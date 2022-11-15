use axum::{
    body::{boxed, Full},
    extract::ws::{WebSocket, WebSocketUpgrade},
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use cloud_console::ConsoleMux;
use futures::{sink::SinkExt, stream::StreamExt};
use rust_embed::RustEmbed;
use tokio::{
    fs::OpenOptions,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, Mutex},
    time,
};
use tower_http::compression::CompressionLayer;

use std::{net::SocketAddr, sync::Arc, time::Duration};

/// 80 columns, 2000 rows. Technically the Mux does not track rows but just a byte array. This is
///    a sane default as such: a single column can contain up to 4 bytes (since it is unicode),
///    however not all rows will be completely filled. Most will indeed only be partially used.
///    As such, 40 bytes per line on average should be rather sufficient.
const CONSOLE_BUFFER: usize = 80 / 2 * 2000;
/// Amount of data fragments from remotes to buffer while forwarding to the pty. If there are more
/// than this amount queued, new writes from remotes will block. Not sure if this is even needed.
const WRITE_BACKLOG: usize = 100;

#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct Assets;

/// Application shared state between handlers.
#[derive(Clone)]
struct State {
    inner: Arc<Mutex<ConsoleMux<CONSOLE_BUFFER>>>,
    data_sender: mpsc::Sender<Vec<u8>>,
}

impl State {
    /// Create a new State with a default iniitalized ConsoleMux and the given channel write half
    /// to forward data to the pty.
    pub fn new(data_sender: mpsc::Sender<Vec<u8>>) -> State {
        State {
            inner: Arc::new(Mutex::new(ConsoleMux::new())),
            data_sender,
        }
    }

    /// Retrieve a reference to the ConsoleMux.
    pub fn console(&self) -> Arc<Mutex<ConsoleMux<CONSOLE_BUFFER>>> {
        self.inner.clone()
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args();
    // Binary name - ignore
    let _ = args.next();
    let pty = if let Some(pty) = args.next() {
        pty
    } else {
        print_usage_and_exit()
    };
    let bind_ip = if let Some(bind_ip) = args.next() {
        bind_ip
    } else {
        print_usage_and_exit()
    };
    let bind_port = if let Some(bind_port) = args.next() {
        bind_port
    } else {
        print_usage_and_exit()
    };
    // Optional log file
    let log_file = args.next();
    let addr = SocketAddr::new(bind_ip.parse().unwrap(), bind_port.parse().unwrap());

    // Open the pty file handle twice, one for reading and one for writing. Opening it in both read
    // + write, then calling `.split()` on it seems to resuld in a deadlock somehow.
    let mut reader = OpenOptions::new()
        .read(true)
        .write(false)
        .create(false)
        .truncate(false)
        .open(&pty)
        .await
        .unwrap();
    let mut writer = OpenOptions::new()
        .read(false)
        .write(true)
        .create(false)
        .truncate(false)
        .open(pty)
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(WRITE_BACKLOG);

    // Loop to forward console data to pty.
    tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            if let Err(e) = writer.write_all(&data).await {
                // Consider this to be fatal
                eprintln!("Could not forward data to pty {}", e);
                // Sleep for a couple of seconds to allow clients to get the latest state of the
                // console mux.
                time::sleep(Duration::from_secs(5)).await;
                std::process::exit(2);
            }
        }
    });

    let state = State::new(tx);
    let console = state.console();
    // Loop to forward pty data to console mux
    tokio::spawn(async move {
        // TODO: good buffer size?
        let mut buffer = [0; 320];
        loop {
            let n = match reader.read(&mut buffer).await {
                Ok(n) => n,
                Err(e) => {
                    // This cleanup is not ideal but sufficient for our usecase
                    eprintln!("Could not read from pty {}", e);
                    // Sleep for a couple of seconds to allow clients to get the latest state of the
                    // console mux.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    std::process::exit(2);
                }
            };
            // Forward data to console mux.
            console.lock().await.write_data(&buffer[..n]);
        }
    });

    // If there is a log file, attach it to the mux to receive the console output as well.
    if let Some(log_file) = log_file {
        let file = OpenOptions::new()
            .read(false)
            .write(true)
            .create(true)
            .truncate(false)
            .open(log_file)
            .await
            .unwrap();
        state.inner.lock().await.attach_remote(file).await;
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/ws", get(handler))
        .fallback(get(static_handler))
        .layer(CompressionLayer::new())
        .layer(Extension(state));

    //tokio::task::spawn(async move {
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handler(ws: WebSocketUpgrade, Extension(state): Extension<State>) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: State) {
    // Split socket in a tx and rx pair.
    let (mut sender, receiver) = socket.split();
    // Attach tx pair to console.
    // Since SplitSink only implements futures-sink::Sink, we need a converter. Do in-memory for
    // now.
    // TODO: good channel capacity;
    let (tx, mut rx) = mpsc::channel::<Arc<Vec<u8>>>(1000);

    tokio::spawn(async move {
        while let Some(buf) = rx.recv().await {
            if let Err(e) = sender
                .send(axum::extract::ws::Message::Binary(buf.to_vec()))
                .await
            {
                eprintln!("Could not send buffer to websocket {}", e);
                // Try to close the socket so the other half is also closed for automatic cleanup.
                // We don't care about errors here
                let _ = sender.close().await;
                return;
            };
        }
    });
    state.inner.lock().await.attach_channel(tx).await;

    tokio::spawn({
        async move {
            receiver
                .for_each(|msg| async {
                    if let Ok(msg) = msg {
                        match msg {
                            axum::extract::ws::Message::Binary(d) => {
                                if let Err(e) = state.data_sender.send(d).await {
                                    eprintln!("Could not send data to pty forwarder {}", e);
                                    return;
                                }
                            }
                            axum::extract::ws::Message::Text(t) => {
                                if let Err(e) = state.data_sender.send(t.into_bytes()).await {
                                    eprintln!("Could not send data to pty forwarder {}", e);
                                    return;
                                }
                            }
                            m => {
                                eprintln!("Unsupported websocket message {:?}", m);
                            }
                        };
                    };
                })
                .await;
        }
    });
}

/// Prints usage instructions to stdandard error, and exists the process with an error code.
fn print_usage_and_exit() -> ! {
    eprintln!(
        r#"Cloud console - An interactive web based terminal connected to a pty
    Usage:
        cloud-console <path_to_pty> <bind_ip> <bind_port> [<log_file>]"#
    );
    std::process::exit(1);
}

/// Handle index
async fn index() -> impl IntoResponse {
    static_handler("/index.html".parse::<Uri>().unwrap()).await
}

/// Handle static files
async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/').to_string();

    StaticFile(path)
}

pub struct StaticFile<T>(pub T);

impl<T> IntoResponse for StaticFile<T>
where
    T: Into<String>,
{
    fn into_response(self) -> Response {
        let path = self.0.into();

        match Assets::get(path.as_str()) {
            Some(content) => {
                let body = boxed(Full::from(content.data));
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                Response::builder()
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(body)
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(boxed(Full::from("404")))
                .unwrap(),
        }
    }
}
