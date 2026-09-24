use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rsa::KeyPair;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::KeyPair as _;
use camino::Utf8PathBuf;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::thread;
use tempfile::TempDir;

struct GeneratedKey {
    pem: String,
    public_der: Vec<u8>,
}

fn generated_key() -> &'static GeneratedKey {
    static KEY: OnceLock<GeneratedKey> = OnceLock::new();
    KEY.get_or_init(|| {
        let pair = KeyPair::generate(KeySize::Rsa2048).expect("RSA key pair generates");
        let der = pair.as_der().expect("private key encodes as PKCS#8");
        GeneratedKey {
            pem: pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref())),
            public_der: pair.public_key().as_ref().to_vec(),
        }
    })
}

pub(crate) fn key_pem() -> &'static str {
    &generated_key().pem
}

pub(crate) fn public_key_der() -> &'static [u8] {
    &generated_key().public_der
}

pub(crate) fn write_key(dir: &TempDir) -> Utf8PathBuf {
    let path =
        Utf8PathBuf::try_from(dir.path().join("app.pem")).expect("temporary directory is UTF-8");
    fs_err::write(&path, key_pem()).expect("private key is written");
    path
}

pub(crate) struct Response {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

impl Response {
    pub(crate) fn json(status: u16, body: impl AsRef<[u8]>) -> Self {
        Self {
            status,
            location: None,
            body: body.as_ref().to_vec(),
        }
    }

    pub(crate) fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            location: Some(location.to_owned()),
            body: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map_or("", |(_, value)| value.as_str())
    }
}

pub(crate) struct Server {
    pub(crate) url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .expect("request log is not poisoned")
            .clone()
    }
}

pub(crate) fn serve(responses: Vec<Response>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback port binds");
    let url = format!(
        "http://{}",
        listener.local_addr().expect("listener has an address")
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&requests);
    thread::spawn(move || {
        for response in responses {
            let (stream, _) = listener.accept().expect("fixture accepts a connection");
            answer(stream, &response, &log);
        }
    });
    Server { url, requests }
}

fn answer(stream: TcpStream, response: &Response, log: &Mutex<Vec<Request>>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("request line is readable");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        line.clear();
        reader
            .read_line(&mut line)
            .expect("header line is readable");
        let Some((name, value)) = line.trim_end().split_once(':') else {
            break;
        };
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    let request = Request {
        method,
        path,
        headers,
    };
    log.lock()
        .expect("request log is not poisoned")
        .push(request);

    let location = response
        .location
        .as_ref()
        .map_or(String::new(), |location| {
            format!("Location: {location}\r\n")
        });
    let head = format!(
        "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{location}\r\n",
        response.status,
        response.body.len(),
    );
    let mut stream = reader.into_inner();
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(&response.body))
        .expect("response is written");
}
