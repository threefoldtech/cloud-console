use std::sync::Arc;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};

const CONNECTION_BUFFER: usize = 1000;

/// An internal console buffer, multiplexing to multiple outputs. The size of the buffer is a
/// constant parameter.
/// The internal buffer is intentionally extremely dumb. In other words, it won't store a certain
/// amount of lines, but rather just an amount of data. It is up to the user to guestimate how much
/// buffer space is needed to keep the required history.
pub struct ConsoleMux<const H: usize> {
    data: [u8; H],
    head: usize,
    remotes: Vec<mpsc::Sender<Arc<Vec<u8>>>>,
}

impl<const H: usize> ConsoleMux<H> {
    /// Create a new ConsoleMux with no data.
    // TODO: implement a cleanup loop for dropped receivers, this now only happens when data is
    // send and can cause a theoretical buildup.
    pub fn new() -> ConsoleMux<H> {
        ConsoleMux {
            data: [0; H],
            head: 0,
            remotes: Vec::new(),
        }
    }

    /// Writes data to the console. The last H bytes of data are retained in the internal buffer
    /// and will be served to new clients when they connect
    pub fn write_data(&mut self, data: &[u8]) {
        // Only keep the data we can actually write to the buffer.
        let sized_data = if data.len() > H {
            &data[data.len() - H..]
        } else {
            data
        };

        // Write data to buffer from head -> end of buffer
        let remainder = H - self.head;
        let to_write = usize::min(remainder, sized_data.len());
        self.data[self.head..self.head + to_write].copy_from_slice(&sized_data[..to_write]);
        // Now write remainder of data to start of buffer -> head
        if sized_data.len() > remainder {
            self.data[..sized_data.len() - remainder].copy_from_slice(&sized_data[to_write..]);
        }

        // Update head
        self.head = (self.head + sized_data.len()) % H;

        // Write data to connected endpoints, but check if there are any first. This avoids a heap
        // allocation if it is not needed.
        if self.remotes.is_empty() {
            return;
        }

        let msg = Arc::new(Vec::from(data));

        // Importantly we do a try send here to avoid blocking. If the channel is full, the remote
        // is lagging and we drop the message. This will likely cause a disconnect and reconnect
        // later. If the remote is disconnected it means it is gone entirely.
        self.remotes.retain(|remote| {
            !matches!(
                remote.try_send(msg.clone()),
                Err(mpsc::error::TrySendError::Closed(_))
            )
        });
    }

    /// Attach a new remote, which will receive data every time a write happens on console mux.
    /// The future returned by this function completes as soon as the existing buffer is sent to
    /// the remote. Errors encountered during writing at any point will not be propagated, instead
    /// they will simply stop the processing of data. After an error writing to the remote, no new
    /// data will be written to the remote.
    ///
    /// # Panics
    ///
    /// This function will panic when executed outside the scope of a [`tokio::runtime::Runtime`]
    pub async fn attach_remote<R>(&mut self, mut remote: R)
    where
        R: AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel(CONNECTION_BUFFER);
        self.remotes.push(tx);

        // Write the contents of the existing buffer
        if let Err(e) = remote.write_all(&self.data[self.head..]).await {
            eprintln!("Error writing first half of data buffer to remote {}", e);
            return;
        }
        if let Err(e) = remote.write_all(&self.data[..self.head]).await {
            eprintln!("Error writing second half of data buffer to remote {}", e);
            return;
        }

        // Spawn data forwarding loop.
        tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                // If we encounter an error writing to the remote, treat it as fatal. Also, use
                // write_all as a convenience here.
                if let Err(e) = remote.write_all(&data).await {
                    eprintln!("Error writing to remote {}", e);
                    break;
                }
            }
        });
    }

    /// Attach a new channel sender to the console, which will be used to notify the receiver of
    /// all sends. The future returned by this function completes as soon as the existing buffer
    /// has been send on the channel. If there is sufficient capacity, this will not wait for the
    /// data to be consumed.
    ///
    /// This function is intended as a convenience function in case the caller does not have a type
    /// compatible with [`AsyncWrite`] to call [`ConsoleMux::attach_remote`].
    ///
    /// # Panics
    ///
    /// This function will panic when executed outside the scope of a [`tokio::runtime::Runtime`]
    pub async fn attach_channel(&mut self, tx: mpsc::Sender<Arc<Vec<u8>>>) {
        // Write the contents of the existing buffer
        if let Err(e) = tx.send(Arc::new(Vec::from(&self.data[self.head..]))).await {
            eprintln!("Error writing first half of data buffer to channel {}", e);
            return;
        }
        if let Err(e) = tx.send(Arc::new(Vec::from(&self.data[..self.head]))).await {
            eprintln!("Error writing second half of data buffer to channel {}", e);
            return;
        }

        self.remotes.push(tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mux_write_no_rotation() {
        let mut cm = ConsoleMux::<200>::new();

        let data = vec![1; 150];
        cm.write_data(&data);

        assert_eq!(cm.head, 150);
        assert_eq!(cm.data[149], 1);
        assert_eq!(cm.data[150], 0);
    }

    #[test]
    fn test_mux_write_with_rotation() {
        let mut cm = ConsoleMux::<200>::new();

        let data = vec![1; 150];
        cm.write_data(&data);

        let data = vec![2; 90];
        cm.write_data(&data);

        assert_eq!(cm.head, 40);
        assert_eq!(cm.data[39], 2);
        assert_eq!(cm.data[40], 1);
        assert_eq!(cm.data[149], 1);
        assert_eq!(cm.data[150], 2);
    }

    #[test]
    fn test_mux_write_large_buffer_no_rotation() {
        let mut cm = ConsoleMux::<100>::new();

        let data = vec![1; 150];
        cm.write_data(&data);

        assert_eq!(cm.head, 0);
        assert_eq!(&cm.data, vec![1; 100].as_slice());
    }

    #[test]
    fn test_mux_write_large_buffer_with_rotation() {
        let mut cm = ConsoleMux::<100>::new();

        let data = vec![1; 50];
        cm.write_data(&data);

        let data = vec![2; 125];
        cm.write_data(&data);

        assert_eq!(cm.head, 50);
        assert_eq!(&cm.data, vec![2; 100].as_slice());
    }
}
