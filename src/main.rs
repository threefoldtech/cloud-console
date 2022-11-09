use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade},
    response::Response,
    routing::get,
    Extension, Router,
};
use futures::{
    sink::SinkExt,
    stream::{SplitSink, SplitStream, StreamExt},
};
use tokio::{
    fs::OpenOptions,
    io::{self, AsyncReadExt, AsyncWriteExt},
};

use std::sync::{Arc, Mutex};

struct State {
    buffer: Mutex<[[u8; 80]; 1000]>,
    row: usize,
    pos: (u8, u8),
    height: usize,
    width: usize,
}

impl State {
    fn new() -> State {
        State {
            buffer: Mutex::new([[0; 80]; 1000]),
            row: 0,
            pos: (0, 0),
            height: 25,
            width: 80,
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args();
    // Binary name - ignore
    let _ = args.next();
    let pty = args.next();
    let bind_ip = args.next();
    let bind_port = args.next();
    //     if pty.is_none() || bind_ip.is_none() || bind_port.is_none() {
    //         eprintln!(
    //             r#"Cloud console - An interactive web based terminal connected to a pty
    // Usage:
    //     cloud-console <path_to_pty> <bind_ip> <bind_port>"#
    //         );
    //         std::process::exit(1);
    //     }
    // let mut handles = Vec::new();

    // handles.push(tokio::spawn(async move {
    //     let mut stdout = io::stdout();
    //     io::copy(&mut reader, &mut stdout).await
    //     //    let mut buf = [0; 80];
    //     //    loop {
    //     //        let n = reader.read(&mut buf).await.unwrap();
    //     //        eprintln!("Read {} bytes from pts", n);
    //     //        stdout.write(&buf[..n]).await.unwrap();
    //     //        eprintln!("Wrote {} bytes to stdout", n);
    //     //    }
    // }));

    // handles.push(tokio::spawn(async move {
    //     let mut stdin = io::stdin();
    //     io::copy(&mut stdin, &mut writer).await
    //     // let mut buf = [0; 80];
    //     // loop {
    //     //     let n = stdin.read(&mut buf).await.unwrap();
    //     //     eprintln!("Read {} bytes from stdin", n);
    //     //     writer.write(&buf[..n]).await.unwrap();
    //     //     eprintln!("Wrote {} bytes to pty", n);
    //     // }
    // }));

    let app = Router::new()
        .route("/ws", get(handler))
        .layer(Extension(Arc::new(State::new())));

    //tokio::task::spawn(async move {
    axum::Server::bind(&"[::]:9999".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
    //});
    // for handle in handles {
    //     handle.await.unwrap().unwrap();
    //     //handle.await.unwrap();
    // }
}

async fn handler(ws: WebSocketUpgrade, Extension(state): Extension<Arc<State>>) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<State>) {
    let (mut sender, receiver) = socket.split();
    tokio::spawn({
        let state = state.clone();
        async move {
            let mut reader = OpenOptions::new()
                .read(true)
                .write(false)
                .create(false)
                .truncate(false)
                .open("/dev/pts/2")
                .await
                .unwrap();
            loop {
                let mut buf = vec![0; 320];
                let n = reader.read(&mut buf).await.unwrap();
                buf.truncate(n);
                sender
                    .send(axum::extract::ws::Message::Binary(buf))
                    .await
                    .unwrap();
            }
        }
    });

    tokio::spawn({
        async move {
            let mut writer = OpenOptions::new()
                .read(false)
                .write(true)
                .create(false)
                .truncate(false)
                .open("/dev/pts/2")
                .await
                .unwrap();
            receiver
                .for_each(|msg| async {
                    let mut writer = writer.try_clone().await.unwrap();
                    if let Ok(msg) = msg {
                        match msg {
                            axum::extract::ws::Message::Binary(d) => {
                                writer.write(&d).await.unwrap();
                            }
                            axum::extract::ws::Message::Text(t) => {
                                writer.write(&t.as_bytes()).await.unwrap();
                            }
                            _ => {}
                        };
                    };
                })
                .await;
        }
    });
}
