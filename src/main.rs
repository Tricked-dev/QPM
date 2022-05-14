use clap::{Args, Parser, Subcommand};

use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::process::Command;
use std::{env, error::Error};
use tokio::net::UdpSocket;
use tokio::{fs, io};
use tokio_util::codec::{BytesCodec, FramedRead, FramedWrite};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", content = "d")]
enum Events {
    Kill(),
    Start(),
    Restart(),
    Success(),
    AddProccess {
        command: String,
        pwd: String,
        args: Vec<String>,
        name: String,
    },
}
#[test]
fn test_event() {
    println!("{}", serde_json::to_string(&Events::Kill()).unwrap())
}

#[derive(Serialize, Deserialize, Debug, Clone)]

struct Proccess {
    id: u32,
    ts: u64,
    name: String,
    command: String,
    args: Vec<String>,
    pwd: String,
    enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Default)]
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

    Ok(socket)
}

async fn get_config() -> Config {
    fs::create_dir_all("~/.qpm/logs").await.unwrap();
    let res = fs::read_to_string("~/.qpm/config.json")
        .await
        .unwrap_or_default();

    serde_json::from_str(&res).unwrap_or_default()
}
async fn set_config(config: Config) {
    fs::create_dir_all("~/.qpm/logs").await.unwrap();
    fs::write(
        "~/.qpm/config.json",
        serde_json::to_string(&config).unwrap(),
    )
    .await
    .expect("Failed to write config!");
}
async fn add_proccess(name: String, command: String, pwd: String, args: Vec<String>) -> Proccess {
    let prc = Proccess {
        id: 0,
        ts: 0,
        name,
        command,
        args,
        pwd,
        enabled: true,
    };
    let mut config = get_config().await;
    config.proccesses.push(prc.clone());
    set_config(config).await;
    prc
}
async fn start_proccess(prc: &Proccess) {
    Command::new(&prc.command)
        .current_dir(&prc.pwd)
        .args(&prc.args)
        .spawn()
        .expect("failed to execute process");
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
            if let Some((size, peer)) = to_send {
                let event: Events = serde_json::from_slice(&buf[..size])?;
                println!("{event:?}");
                match event {
                    Events::Kill() => {
                        socket
                            .send_to(&serde_json::to_vec_pretty(&Events::Success())?, &peer)
                            .await?;
                        std::process::exit(0);
                    }
                    Events::AddProccess {
                        command,
                        pwd,
                        args,
                        name,
                    } => {
                        let prc = add_proccess(name, command, pwd, args).await;
                        tokio::spawn(async move {
                            start_proccess(&prc).await;
                        });
                    }
                    _ => {}
                }
                // let amt = socket.send_to(&buf[..size], &peer).await?;
            }

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

const MAX_DATAGRAM_SIZE: usize = 65_507;

#[derive(Args)]
struct Start {
    command: String,
    args: Vec<String>,
    #[clap(short, long)]
    pwd: Option<String>,
    #[clap(short, long)]
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let res = Cli::parse();
    let addr = "127.0.0.1:8080";
    match res.command {
        Commands::Server(Server {}) => {
            let socket = UdpSocket::bind(addr).await?;
            println!("Listening on: {}", socket.local_addr()?);

            for prc in get_config().await.proccesses {
                tokio::spawn(async move {
                    start_proccess(&prc).await;
                });
            }

            let server = UdpServer {
                socket,
                buf: vec![0; 1024],
                to_send: None,
            };

            server.run().await?;
        }
        Commands::Kill(..) => {
            let addr = addr.parse::<SocketAddr>()?;
            let sock = connect(&addr).await?;
            sock.send(&serde_json::to_vec(&Events::Kill())?).await?;

            let mut buf = vec![0u8; MAX_DATAGRAM_SIZE];
            let res = sock.recv(&mut buf).await?;
            println!("{}", String::from_utf8_lossy(&buf));
            println!(
                "{:?}",
                serde_json::from_str::<Events>(&String::from_utf8_lossy(&buf).trim())?
            );
        }
        Commands::Start(Start {
            command,
            pwd,
            name,
            args,
        }) => {
            let addr = addr.parse::<SocketAddr>()?;
            let sock = connect(&addr).await?;
            sock.send(&serde_json::to_vec(&Events::AddProccess {
                name,
                command,
                pwd: pwd.unwrap_or_else(|| {
                    env::current_dir()
                        .unwrap()
                        .as_os_str()
                        .to_string_lossy()
                        .to_string()
                }),
                args: args,
            })?)
            .await?;
        }
    }

    Ok(())
}
