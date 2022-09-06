#![deny(warnings)]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use http::Uri;
use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Body, Client, Method, Request, Response, Server};

use pacparser::{decode_proxy, PacParser, ProxyEntry, ProxyType};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

type HttpClient = Client<hyper::client::HttpConnector>;

#[derive(Parser)]
struct Args {
    #[clap(short, long)]
    pac_file: PathBuf,
}

// To try this example:
// 1. cargo run --example http_proxy
// 2. config http_proxy in command line
//    $ export http_proxy=http://127.0.0.1:8100
//    $ export https_proxy=http://127.0.0.1:8100
// 3. send requests
//    $ curl -i https://www.some_domain.com/
#[tokio::main]
async fn main() {
    let args = Args::from_args();

    let addr = SocketAddr::from(([127, 0, 0, 1], 8100));

    let client = Client::builder()
        .http1_title_case_headers(true)
        .http1_preserve_header_case(true)
        .build_http();

    let (pac_sender, mut pac_recv): (PacSender, _) = mpsc::channel(128);
    let local_pool = tokio_util::task::LocalPoolHandle::new(1);
    local_pool.spawn_pinned(|| async move {
        let mut pac_lib = PacParser::new()?;
        let mut pac_file = pac_lib.load_path(args.pac_file)?;

        while let Some((url, rsp)) = pac_recv.recv().await {
            let proxy = pac_file.find_proxy(
                &url.to_string(),
                url.host().unwrap_or("NO HOST, WHAT TO DO?"),
            )?;

            let decoded = decode_proxy(proxy)?;
            let _ = rsp.send(decoded);
        }

        Ok::<_, pacparser::Error>(())
    });

    let make_service = make_service_fn(move |_| {
        let client = client.clone();
        let pac_sender = pac_sender.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                proxy(client.clone(), pac_sender.clone(), req)
            }))
        }
    });

    let server = Server::bind(&addr)
        .http1_preserve_header_case(true)
        .http1_title_case_headers(true)
        .serve(make_service);

    println!("Listening on http://{}", addr);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

type PacSender = mpsc::Sender<(Uri, oneshot::Sender<Vec<ProxyEntry>>)>;

async fn proxy(
    client: HttpClient,
    pac_sender: PacSender,
    req: Request<Body>,
) -> Result<Response<Body>, hyper::Error> {
    let uri = req.uri().clone();
    let (send, recv) = oneshot::channel();

    pac_sender
        .send((uri, send))
        .await
        .expect("PAC task errored");

    let proxies = recv.await.expect("PAC task exited");

    if Method::CONNECT == req.method() {
        // Received an HTTP request like:
        // ```
        // CONNECT www.domain.com:443 HTTP/1.1
        // Host: www.domain.com:443
        // Proxy-Connection: Keep-Alive
        // ```
        //
        // When HTTP method is CONNECT we should return an empty body
        // then we can eventually upgrade the connection and talk a new protocol.
        //
        // Note: only after client received an empty body with STATUS_OK can the
        // connection be upgraded, so we can't return a response inside
        // `on_upgrade` future.
        for entry in proxies {
            match entry {
                ProxyEntry::Direct => {
                    return if let Some(addr) = host_addr(req.uri()) {
                        tokio::task::spawn(async move {
                            match hyper::upgrade::on(req).await {
                                Ok(upgraded) => {
                                    if let Err(e) = tunnel(upgraded, addr).await {
                                        eprintln!("server io error: {}", e);
                                    };
                                }
                                Err(e) => eprintln!("upgrade error: {}", e),
                            }
                        });

                        Ok(Response::new(Body::empty()))
                    } else {
                        eprintln!("CONNECT host is not socket addr: {:?}", req.uri());
                        let mut resp =
                            Response::new(Body::from("CONNECT must be to a socket address"));
                        *resp.status_mut() = http::StatusCode::BAD_REQUEST;

                        Ok(Response::new(Body::empty()))
                    }
                }
                ProxyEntry::Proxied { ty, host, port } => match ty {
                    ProxyType::Proxy | ProxyType::Http => todo!("proxy through {}, {}", host, port),
                    _ => panic!("ProxyType not supported: {:?}", ty),
                },
            }
        }

        todo!("No route found")
    } else {
        for entry in proxies {
            match entry {
                ProxyEntry::Direct => return client.request(req).await,
                ProxyEntry::Proxied { ty, host, port } => match ty {
                    ProxyType::Proxy | ProxyType::Http => todo!("proxy through {}, {}", host, port),
                    _ => panic!("ProxyType not supported: {:?}", ty),
                },
            }
        }

        todo!("No route, still trying direct");
    }
}

fn host_addr(uri: &http::Uri) -> Option<String> {
    uri.authority().map(|auth| auth.to_string())
}

// Create a TCP connection to host:port, build a tunnel between the connection and
// the upgraded connection
async fn tunnel(mut upgraded: Upgraded, addr: String) -> std::io::Result<()> {
    // Connect to remote server
    let mut server = TcpStream::connect(addr).await?;

    // Proxying data
    tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;

    Ok(())
}
