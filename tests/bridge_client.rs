use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use grove::bridge::protocol::{
    BridgeCommand, BridgeRequestEnvelope, BridgeResponse, BridgeResponseEnvelope,
};

#[test]
fn bridge_client_sends_json_request_and_reads_json_response() {
    let socket_path = make_socket_path("grove-bridge-client");
    let listener = UnixListener::bind(&socket_path).expect("test socket should bind");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("bridge client should connect");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("server stream clone should succeed"),
        );
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .expect("server should read request line");

        let request: BridgeRequestEnvelope =
            serde_json::from_str(&request_line).expect("request should deserialize");
        assert!(matches!(request.command, BridgeCommand::Ping));

        let response = BridgeResponseEnvelope {
            request_id: request.request_id,
            response: BridgeResponse::Pong,
        };
        let encoded = serde_json::to_string(&response).expect("response should serialize");
        stream
            .write_all(encoded.as_bytes())
            .expect("server should write response");
        stream
            .write_all(b"\n")
            .expect("server should terminate response line");
    });

    let mut client = grove::bridge::client::BridgeClient::new(socket_path.clone());
    let response = client
        .send_command(BridgeCommand::Ping)
        .expect("client round trip should succeed");

    assert!(matches!(response, BridgeResponse::Pong));

    server.join().expect("server thread should join");
    fs::remove_file(socket_path).expect("test socket should be removed");
}

fn make_socket_path(prefix: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{suffix}.sock"))
}
