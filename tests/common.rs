#![allow(dead_code)]

extern crate fantoccini;
extern crate futures_util;

use fantoccini::error;

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use warp::Filter;
use fantoccini::ClientBuilder;
use fantoccini::Client;

#[cfg(feature = "rustls-tls")]
pub async fn rustls_type(s: &str) -> Result<Client<hyper_rustls::HttpsConnector<hyper::client::HttpConnector>>, error::NewSessionError> {
    match s {
        "firefox" => {
            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({ "args": ["--headless"] });
            caps.insert("moz:firefoxOptions".to_string(), opts.clone());
            ClientBuilder::rustls().capabilities(caps).connect("http://localhost:4444").await
        }
        "chrome" => {
            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({
                "args": ["--headless", "--disable-gpu", "--no-sandbox", "--disable-dev-shm-usage"],
                "binary":
                    if std::path::Path::new("/usr/bin/chromium-browser").exists() {
                        // on Ubuntu, it's called chromium-browser
                        "/usr/bin/chromium-browser"
                    } else if std::path::Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome").exists() {
                        // macOS
                        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
                    } else {
                        // elsewhere, it's just called chromium
                        "/usr/bin/chromium"
                    }
            });
            caps.insert("goog:chromeOptions".to_string(), opts.clone());
            ClientBuilder::rustls().capabilities(caps).connect("http://localhost:9515").await
        }
        browser => unimplemented!("unsupported browser backend {}", browser),
    }
}

#[cfg(feature = "openssl-tls")]
pub async fn openssl_type(s: &str) -> Result<Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>, error::NewSessionError> {
    match s {
        "firefox" => {
            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({ "args": ["--headless"] });
            caps.insert("moz:firefoxOptions".to_string(), opts.clone());
            ClientBuilder::openssl().capabilities(caps).connect("http://localhost:4444").await
        }
        "chrome" => {
            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({
                "args": ["--headless", "--disable-gpu", "--no-sandbox", "--disable-dev-shm-usage"],
                "binary":
                    if std::path::Path::new("/usr/bin/chromium-browser").exists() {
                        // on Ubuntu, it's called chromium-browser
                        "/usr/bin/chromium-browser"
                    } else if std::path::Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome").exists() {
                        // macOS
                        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
                    } else {
                        // elsewhere, it's just called chromium
                        "/usr/bin/chromium"
                    }
            });
            caps.insert("goog:chromeOptions".to_string(), opts.clone());
            ClientBuilder::openssl().capabilities(caps).connect("http://localhost:9515").await
        }
        browser => unimplemented!("unsupported browser backend {}", browser),
    }
}


pub fn handle_test_error(
    res: Result<Result<(), fantoccini::error::CmdError>, Box<dyn std::any::Any + Send>>,
) -> bool {
    match res {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            eprintln!("test future failed to resolve: {:?}", e);
            false
        }
        Err(e) => {
            if let Some(e) = e.downcast_ref::<error::CmdError>() {
                eprintln!("test future panicked: {:?}", e);
            } else if let Some(e) = e.downcast_ref::<error::NewSessionError>() {
                eprintln!("test future panicked: {:?}", e);
            } else {
                eprintln!("test future panicked; an assertion probably failed");
            }
            false
        }
    }
}


#[cfg(feature = "rustls-tls")]
#[macro_export]
macro_rules! rustls_tester {
    ($f:ident, $endpoint:expr) => {{
        use std::sync::{Arc, Mutex};
        use std::thread;

        let c = common::rustls_type($endpoint);

        // we'll need the session_id from the thread
        // NOTE: even if it panics, so can't just return it
        let session_id = Arc::new(Mutex::new(None));

        // run test in its own thread to catch panics
        let sid = session_id.clone();
        let res = thread::spawn(move || {
            let mut rt = tokio::runtime::Builder::new()
                .enable_all()
                .basic_scheduler()
                .build()
                .unwrap();
            let mut c = rt.block_on(c).expect("failed to construct test client");
            *sid.lock().unwrap() = rt.block_on(c.session_id()).unwrap();
            // make sure we close, even if an assertion fails
            let x = rt.block_on(async move {
                let r = tokio::spawn($f(c.clone())).await;
                let _ = c.close().await;
                r
            });
            drop(rt);
            x.expect("test panicked")
        })
        .join();
        let success = common::handle_test_error(res);
        assert!(success);
    }};
}


#[cfg(feature = "openssl-tls")]
#[macro_export]
macro_rules! openssl_tester {
    ($f:ident, $endpoint:expr) => {{
        use std::sync::{Arc, Mutex};
        use std::thread;

        let c = common::openssl_type($endpoint);

        // we'll need the session_id from the thread
        // NOTE: even if it panics, so can't just return it
        let session_id = Arc::new(Mutex::new(None));

        // run test in its own thread to catch panics
        let sid = session_id.clone();
        let res = thread::spawn(move || {
            let mut rt = tokio::runtime::Builder::new()
                .enable_all()
                .basic_scheduler()
                .build()
                .unwrap();
            let mut c = rt.block_on(c).expect("failed to construct test client");
            *sid.lock().unwrap() = rt.block_on(c.session_id()).unwrap();
            // make sure we close, even if an assertion fails
            let x = rt.block_on(async move {
                let r = tokio::spawn($f(c.clone())).await;
                let _ = c.close().await;
                r
            });
            drop(rt);
            x.expect("test panicked")
        })
        .join();
        let success = common::handle_test_error(res);
        assert!(success);
    }};
}

#[cfg(feature = "rustls-tls")]
#[macro_export]
macro_rules! rustls_local {
    ($f:ident, $endpoint:expr) => {{
        let port: u16 = common::setup_server();
        let f = move |c: Client<hyper_rustls::HttpsConnector<hyper::client::HttpConnector>>| async move { $f(c, port).await };
        rustls_tester!(f, $endpoint)
    }};
}

#[cfg(feature = "openssl-tls")]
#[macro_export]
macro_rules! openssl_local {
    ($f:ident, $endpoint:expr) => {{
        let port: u16 = common::setup_server();
        let f = move |c: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>| async move { $f(c, port).await };
        openssl_tester!(f, $endpoint)
    }};
}

/// Sets up the server and returns the port it bound to.
pub fn setup_server() -> u16 {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let mut rt = tokio::runtime::Builder::new()
            .enable_all()
            .basic_scheduler()
            .build()
            .unwrap();
        let _ = rt.block_on(async {
            let (socket_addr, server) = start_server();
            tx.send(socket_addr.port())
                .expect("To be able to send port");
            server.await
        });
    });

    rx.recv().expect("To get the bound port.")
}

/// Configures and starts the server
fn start_server() -> (SocketAddr, impl Future<Output = ()> + 'static) {
    let socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    const ASSETS_DIR: &str = "tests/test_html";
    let assets_dir: PathBuf = PathBuf::from(ASSETS_DIR);
    let routes = fileserver(assets_dir);
    warp::serve(routes).bind_ephemeral(socket_addr)
}

/// Serves files under this directory.
fn fileserver(
    assets_dir: PathBuf,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::get()
        .and(warp::fs::dir(assets_dir))
        .and(warp::path::end())
}
