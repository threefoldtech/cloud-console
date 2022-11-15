# Cloud console

Cloud console is a small application intended to provide a web based terminal for virtual machines, who make a serial device or virtio-console
available over a `pseudoterminal` (`pty`). Although primarily intended to be used with [cloud-hypervisor], any vmm which exposes a `pty` (or
any application which exposes a `pty` for that matter) should be compatible.

## Implementation

The current system connects to the `pty` for both reading and writing. The read end is plugged into a multiplexer, with an internal buffer.
Clients connect, and their write half is connected to the same multiplexer. On connection, the entirety of the buffer is send to the client.
This way, clients can see a some history about the session once they connect. Once a write is done on the `pty` by the guest, this guest is
propagated to the multiplexer, included in the buffer, and then sent to every connected client. These clients maintain a small internal buffer
for writes as well. Should the buffer be full (because of a laggy client for instance), the message is dropped. If this is noticed by the consumer,
they should reconnect.

The read half of connected clients is connected with an internal process, which forwards input from all writes to the write half of the `pty`. This
setup allows multiple clients to share the same session. Writes on a session are simply propagated to the `pty`, and we rely on the console of the guest
to properly echo the data back to connected clients (including the client who sent the data).

Data propagation happens over a simple websocket protocol. The current protocol is not considered stable and can change between versions without
any backward compatibility.

## Building

Since the frontend is statically included in the final binary, it must be build first. Building the frontend is explained in [the readme of the frontend](./frontend/README.md#building).
Once this is done, you can use default rust commands in this directory to build, e.g.:

```bash
cargo build
```

In debug mode, the frontend files are not compiled into the binary, and changing them (i.e. building them again) will cause the new files to be served
without requiring the binary to be restarted.

To build a static linux release binary, run

```bash
cargo build --release --target x86_64-unknown-linux-musl
```

## Running

The binary expects at least 3 arguments, with an optional 4th:

```bash
cloud-console <path_to_pty> <bind_ip> <bind_port> [<log_file>]
```

- `path_to_pty`: The path to the `pty` device file to connect to
- `bind_ip`: The IP address to bind the server to
- `bind_port`: The port to use for the server
- `log_file`: This is optional, if it is set, this file will be opened (created if needed), and attached as reader to the multiplexer. All data sent by
	the `pty` will be written in the file. Can be used for debug purposed.

## Contributing

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as below, without any additional terms or conditions.

## License

&copy; 2022 Lee Smet

This project is licensed under either of

- [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0) ([`LICENSE-APACHE`](LICENSE-APACHE))
- [MIT license](https://opensource.org/licenses/MIT) ([`LICENSE-MIT`](LICENSE-MIT))

at your option.

The [SPDX](https://spdx.dev) license identifier for this project is `MIT OR Apache-2.0`.

[cloud-hypervisor]: https://github.com/cloud-hypervisor/cloud-hypervisor
