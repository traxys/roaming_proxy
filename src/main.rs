use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use color_eyre::eyre::Context;
use http::{HeaderValue, StatusCode};
use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Client, Method, Request, Response, Server};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

type HttpClient = Client<hyper::client::HttpConnector>;

#[derive(Serialize, Deserialize)]
struct Config {
    v4: HashMap<ipnet::Ipv4Net, String>,
    v6: HashMap<ipnet::Ipv6Net, String>,
}

#[derive(Parser)]
struct Args {
    #[clap(short, long)]
    config: PathBuf,
    #[clap(short, long, default_value = "8100")]
    port: u16,
}

// To try this example:
// 1. cargo run --example http_proxy
// 2. config http_proxy in command line
//    $ export http_proxy=http://127.0.0.1:8100
//    $ export https_proxy=http://127.0.0.1:8100
// 3. send requests
//    $ curl -i https://www.some_domain.com/
#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::from_args();

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));

    let client = Client::builder()
        .http1_title_case_headers(true)
        .http1_preserve_header_case(true)
        .build_http();

    let config: Config = toml::from_str(&std::fs::read_to_string(args.config)?)?;
    let config = Arc::new(config);

    let make_service = make_service_fn(move |_| {
        let client = client.clone();
        let config = config.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let client = client.clone();
                let config = config.clone();
                async move {
                    proxy(client, config, req).await.map_err(|e| {
                        println!("Err: {:?}", e);
                        e
                    })
                }
            }))
        }
    });

    let server = Server::bind(&addr)
        .http1_preserve_header_case(true)
        .http1_title_case_headers(true)
        .serve(make_service);

    println!("Listening on http://{}", addr);

    Ok(server.await?)
}

async fn proxy(
    client: HttpClient,
    config: Arc<Config>,
    req: Request<hyper::Body>,
) -> color_eyre::Result<Response<hyper::Body>> {
    let ip = local_ip_address::local_ip()?;

    let proxy = match ip {
        IpAddr::V4(addr) => config
            .v4
            .iter()
            .find(|(net, _)| net.contains(&addr))
            .map(|(_, host)| host)
            .cloned(),
        IpAddr::V6(addr) => config
            .v6
            .iter()
            .find(|(net, _)| net.contains(&addr))
            .map(|(_, host)| host)
            .cloned(),
    };

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

        let addr = match host_addr(req.uri()) {
            Some(addr) => addr,
            None => {
                eprintln!("CONNECT host is not socket addr: {:?}", req.uri());
                let mut resp =
                    Response::new(hyper::Body::from("CONNECT must be to a socket address"));
                *resp.status_mut() = http::StatusCode::BAD_REQUEST;

                return Ok(Response::new(hyper::Body::empty()));
            }
        };

        match proxy {
            None => {
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

                Ok(Response::new(hyper::Body::empty()))
            }
            Some(host) => {
                tokio::spawn(async {
                    if let Err(e) = double_tunnel(req, addr, host).await {
                        println!("Double tunnel errored: {:?}", e)
                    }
                });

                Ok(Response::new(hyper::Body::empty()))
            }
        }
    } else {
        match proxy {
            None => client.request(req).await.map_err(Into::into),
            Some(host) => {
                let distant = TcpStream::connect(&host).await?;
                let (mut req_sender, conn) = hyper::client::conn::handshake(distant).await?;

                tokio::spawn(async move {
                    if let Err(e) = conn.await {
                        eprintln!("Error in connection: {}", e);
                    }
                });
                req_sender.send_request(req).await.map_err(Into::into)
            }
        }
    }
}

async fn double_tunnel(
    req: Request<hyper::Body>,
    addr: String,
    host: String,
) -> color_eyre::Result<()> {
    let distant_connect = Request::connect(req.uri())
        .header("host", addr)
        .header(
            "user-agent",
            req.headers()
                .get("user-agent")
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_str("pac_proxy").unwrap()),
        )
        .header("proxy-connection", "Keep-Alive")
        .body(hyper::Body::empty())?;

    let distant = TcpStream::connect(&host)
        .await
        .with_context(|| format!("Could not connect to distant {host}"))?;

    let (mut req_sender, conn) = hyper::client::conn::handshake(distant).await?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            println!("Connection failed: {e:?}");
        }
    });

    let response = req_sender.send_request(distant_connect).await?;
    if response.status() != StatusCode::OK {
        color_eyre::eyre::bail!("Server did not accept to connect: {:?}", response)
    };

    let mut upgraded_to_proxy = hyper::upgrade::on(response).await?;
    let mut upgraded_client = hyper::upgrade::on(req).await?;

    tokio::io::copy_bidirectional(&mut upgraded_to_proxy, &mut upgraded_client)
        .await
        .context("could not copy in tunnel")?;

    Ok(())
}

fn host_addr(uri: &http::Uri) -> Option<String> {
    uri.authority().map(|auth| auth.to_string())
}

// Create a TCP connection to host:port, build a tunnel between the connection and
// the upgraded connection
async fn tunnel(mut upgraded: Upgraded, addr: String) -> color_eyre::Result<()> {
    // Connect to remote server
    let mut server = TcpStream::connect(addr)
        .await
        .context("could not connect to server")?;

    // Proxying data
    tokio::io::copy_bidirectional(&mut upgraded, &mut server)
        .await
        .context("could not copy in tunnel")?;

    Ok(())
}
