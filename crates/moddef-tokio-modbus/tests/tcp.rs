// SPDX-License-Identifier: Apache-2.0

//! Adapter integration test against an in-process tokio-modbus TCP server:
//! chunked reads honoring max_read_words, write paths, exception mapping,
//! and driving the moddef-core dynamic facade end-to-end over real TCP.

use std::future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use moddef_core::Transport;
use moddef_tokio_modbus::{Options, TokioModbusError, TokioModbusTransport};
use tokio::net::TcpListener;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server};
use tokio_modbus::{ExceptionCode, Request, Response};

#[derive(Default)]
struct Data {
    holding: Vec<u16>,
    input: Vec<u16>,
    read_requests: usize,
}

#[derive(Clone)]
struct DataService(Arc<Mutex<Data>>);

impl tokio_modbus::server::Service for DataService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = future::Ready<Result<Response, ExceptionCode>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let mut d = self.0.lock().unwrap();
        let window = |src: &[u16], addr: u16, cnt: u16| -> Result<Vec<u16>, ExceptionCode> {
            let s = addr as usize;
            let e = s + cnt as usize;
            if e > src.len() {
                return Err(ExceptionCode::IllegalDataAddress);
            }
            Ok(src[s..e].to_vec())
        };
        future::ready(match req {
            Request::ReadHoldingRegisters(addr, cnt) => {
                d.read_requests += 1;
                window(&d.holding, addr, cnt).map(Response::ReadHoldingRegisters)
            }
            Request::ReadInputRegisters(addr, cnt) => {
                d.read_requests += 1;
                window(&d.input, addr, cnt).map(Response::ReadInputRegisters)
            }
            Request::WriteMultipleRegisters(addr, words) => {
                let s = addr as usize;
                if s + words.len() > d.holding.len() {
                    Err(ExceptionCode::IllegalDataAddress)
                } else {
                    d.holding[s..s + words.len()].copy_from_slice(&words);
                    Ok(Response::WriteMultipleRegisters(addr, words.len() as u16))
                }
            }
            _ => Err(ExceptionCode::IllegalFunction),
        })
    }
}

/// Spawn a Modbus TCP server on an ephemeral port, returning its address.
async fn spawn_server(data: Arc<Mutex<Data>>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let server = Server::new(listener);
        let on_connected = move |stream, socket_addr| {
            let data = data.clone();
            async move {
                accept_tcp_connection(stream, socket_addr, move |_| {
                    Ok(Some(DataService(data.clone())))
                })
            }
        };
        server
            .serve(&on_connected, |e| eprintln!("server error: {e}"))
            .await
            .unwrap();
    });
    addr
}

#[tokio::test]
async fn chunked_reads_writes_and_exceptions() {
    let data = Arc::new(Mutex::new(Data {
        holding: (0..300).collect(),
        input: vec![7; 64],
        read_requests: 0,
    }));
    let addr = spawn_server(data.clone()).await;

    let mut t = TokioModbusTransport::tcp(
        addr,
        Options {
            max_read_words: 100,
            ..Options::default()
        },
    )
    .await
    .unwrap();

    // 250 words with max_read_words=100 → 3 requests, data intact.
    let mut regs = vec![0u16; 250];
    t.read_holding(10, &mut regs).await.unwrap();
    assert_eq!(regs[0], 10);
    assert_eq!(regs[249], 259);
    assert_eq!(data.lock().unwrap().read_requests, 3);
    assert_eq!(t.max_read_words(), 100);

    let mut one = [0u16; 1];
    t.read_input(63, &mut one).await.unwrap();
    assert_eq!(one, [7]);

    t.write_holding(5, &[42, 43]).await.unwrap();
    assert_eq!(data.lock().unwrap().holding[5..7], [42, 43]);

    // Out-of-range read surfaces the device's exception code.
    let mut oor = [0u16; 4];
    match t.read_holding(299, &mut oor).await {
        Err(TokioModbusError::Exception(ExceptionCode::IllegalDataAddress)) => {}
        other => panic!("expected IllegalDataAddress, got {other:?}"),
    }
}

#[tokio::test]
async fn dynamic_facade_over_tcp() {
    const DOC: &str = r#"{
      "docId": "test.tcp",
      "version": "1.0.0",
      "devices": [{
        "deviceId": "meter",
        "blocks": [{
          "blockId": "live",
          "space": "HOLDING_REGISTER",
          "lengthWords": 8,
          "points": [{
            "pointId": "voltage",
            "access": "READ_ONLY",
            "storageType": "U16",
            "valueType": {"primitive": "DECIMAL"},
            "mapping": {"space": "HOLDING_REGISTER", "offset": 2, "lengthWords": 1},
            "transform": {"scale": {"numerator": "1", "denominator": "10"}}
          }]
        }]
      }]
    }"#;

    let data = Arc::new(Mutex::new(Data {
        holding: vec![0, 0, 2305, 0, 0, 0, 0, 0],
        input: vec![],
        read_requests: 0,
    }));
    let addr = spawn_server(data).await;
    let t = TokioModbusTransport::tcp(addr, Options::default())
        .await
        .unwrap();

    let doc =
        moddef_core::parse_document(DOC.as_bytes(), moddef_core::DocumentFormat::Json).unwrap();
    let mut dev = moddef_core::Device::new(&doc, Some("meter"), t).unwrap();
    let v = dev.read_point("voltage").await.unwrap();
    assert_eq!(v.as_f64(), Some(230.5));
}
