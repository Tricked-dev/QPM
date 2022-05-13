use clap::{Args, Parser, Subcommand};

use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::process::Command;
use std::{env, error::Error};
use tokio::io;
use tokio::net::UdpSocket;
use tokio_util::codec::{BytesCodec, FramedRead, FramedWrite};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", content = "d")]
enum Events {
    Kill(),
    Start(),
    Restart(),
    AddProccess {
        command: String,
        pwd: String,
        args: Vec<String>,
    },
}
#[derive(Serialize, Deserialize, Debug)]

struct Proccess {
    id: u32,
    ts: u64,
    name: String,
    command: String,
    args: Vec<String>,
    pwd: String,
    enabled: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    proccesses: Vec<Proccess>,
}

async fn connect(addr: &SocketAddr) -> Result<UdpSocket, Box<dyn Error>> {
    let bind_addr = if addr.ip().is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };

    let socket = UdpSocket::bind(&bind_addr).await?;
    socket.connect(addr).await?;
    // socket.send(&serde_json::to_vec(&Events::Kill())?).await?;

    // recv(stdout, &socket).await?;

    Ok(socket)
}

async fn add_proccess(command: String, pwd: String, args: Vec<String>) -> Proccess {
    let prc = Proccess {
        id: 0,
        ts: 0,
        name: "".to_string(),
        command: command,
        args: args,
        pwd: pwd,
        enabled: true,
    };

    prc
}
async fn start_proccess(prc: &Proccess) {
    Command::new(&prc.command)
        .current_dir(&prc.pwd)
        .args(&prc.args)
        .spawn()
        .expect("failed to execute process");
}
async fn send(
    mut stdin: impl Stream<Item = Result<Bytes, io::Error>> + Unpin,
    writer: &UdpSocket,
) -> Result<(), io::Error> {
    while let Some(item) = stdin.next().await {
        let buf = item?;
        writer.send(&buf[..]).await?;
    }

    Ok(())
}

async fn recv(
    mut stdout: impl Sink<Bytes, Error = io::Error> + Unpin,
    reader: &UdpSocket,
) -> Result<(), io::Error> {
    loop {
        let mut buf = vec![0; 1024];
        let n = reader.recv(&mut buf[..]).await?;

        if n > 0 {
            stdout.send(Bytes::from(buf)).await?;
        }
    }
}

struct UdpServer {
    socket: UdpSocket,
    buf: Vec<u8>,
    to_send: Option<(usize, SocketAddr)>,
}

impl UdpServer {
    async fn run(self) -> Result<(), io::Error> {
        let Self {
            socket,
            mut buf,
            mut to_send,
        } = self;

        loop {
            // First we check to see if there's a message we need to echo back.
            // If so then we try to send it back to the original source, waiting
            // until it's writable and we're able to do so.
            if let Some((size, peer)) = to_send {
                let event: Events = serde_json::from_slice(&buf[..size])?;
                println!("{event:?}");
                match event {
                    Events::Kill() => {
                        std::process::exit(0);
                    }
                    Events::AddProccess { command, pwd, args } => {
                        let prc = add_proccess(command, pwd, args).await;
                        start_proccess(&prc).await;
                    }
                    _ => {}
                }
                let amt = socket.send_to(&buf[..size], &peer).await?;
            }

            // If we're here then `to_send` is `None`, so we take a look for the
            // next message we're going to echo back.
            to_send = Some(socket.recv_from(&mut buf).await?);
        }
    }
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Server(Server),
    Kill(Kill),
    Start(Start),
}

#[derive(Args)]
struct Server {}

#[derive(Args)]
struct Kill {}

#[derive(Args)]
struct Start {
    command: String,
    #[clap(short, long)]
    pwd: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let res = Cli::parse();
    let addr = "127.0.0.1:8080";
    match res.command {
        Commands::Server(Server {}) => {
            let socket = UdpSocket::bind(addr).await?;
            println!("Listening on: {}", socket.local_addr()?);

            let server = UdpServer {
                socket,
                buf: vec![0; 1024],
                to_send: None,
            };

            // This starts the server task.
            server.run().await?;
        }
        Commands::Kill(..) => {
            let addr = addr.parse::<SocketAddr>()?;
            let sock = connect(&addr).await?;
            sock.send(&serde_json::to_vec(&Events::Kill())?).await?;
            // let stdin = FramedRead::new(io::stdin(), BytesCodec::new());
            // let stdin = stdin.map(|i| i.map(|bytes| bytes.freeze()));
            // let stdout = FramedWrite::new(io::stdout(), BytesCodec::new());
            // let addr = addr.parse::<SocketAddr>()?;

            // connect(&addr, stdin, stdout).await?;
        }
        Commands::Start(Start { command, pwd }) => {
            let addr = addr.parse::<SocketAddr>()?;
            let sock = connect(&addr).await?;
            sock.send(&serde_json::to_vec(&Events::AddProccess {
                command,
                pwd: pwd.unwrap_or_else(|| {
                    env::current_dir()
                        .unwrap()
                        .as_os_str()
                        .to_string_lossy()
                        .to_string()
                }),
                args: vec![],
            })?)
            .await?;
        }
    }

    Ok(())
}
