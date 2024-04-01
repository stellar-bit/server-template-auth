use futures::prelude::sink::SinkExt;
use tokio::time::interval;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;
use stellar_bit_core::prelude::*;
use stellar_bit_core::{
    game::GameCmdExecutionError,
    network::{ClientRequest, ServerResponse},
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};

mod client_handle;
use client_handle::ClientHandle;

mod hub_connection;
pub use hub_connection::ServerHubConn;


pub const SERVER_ADDR: SocketAddrV4 = todo!(); // public server address including port
pub const SERVER_NAME: &str = todo!();

pub async fn start_server(hub_conn: ServerHubConn) {
    let game_session = GameSession::new();

    let game_clone = game_session.game.clone();
    tokio::task::spawn(run_server(game_clone, hub_conn));

    game_session.game_loop(200).unwrap();
}


struct GameSession {
    game: Arc<RwLock<Game>>,
}

impl GameSession {
    pub fn new() -> Self {
        Self {
            game: Arc::new(RwLock::new(Game::new())),
        }
    }
    pub fn game_loop(self, fps: u32) -> Result<(), GameCmdExecutionError> {
        let target_duration = std::time::Duration::from_secs_f32(1. / fps as f32);

        loop {
            let frame_time_measure = std::time::Instant::now();

            let mut game = self.game.write().unwrap();

            let dt = now() - game.sync.last_update;
            game.update(dt.as_secs_f32());

            drop(game);

            let frame_time = frame_time_measure.elapsed();
            if frame_time < target_duration {
                std::thread::sleep(target_duration - frame_time);
            } else {
                eprintln!(
                    "Server is behind intended frame rate, delay: {} ms",
                    (frame_time - target_duration).as_millis()
                )
            }
        }
    }
}

async fn run_server(game: Arc<RwLock<Game>>, hub_conn: ServerHubConn) {
    let hub_conn = Arc::new(hub_conn);
    // keep alive task
    {
        let hub_conn_c = hub_conn.clone();
        tokio::spawn(async move {
            let hub_conn = hub_conn_c;
            let mut ka_interval = interval(Duration::from_secs(60*3));
            loop {
                ka_interval.tick().await;
                println!("Sending keep alive!");
                hub_conn.keep_alive().await;
            }
        });
    }

    let server_address = SERVER_ADDR;
    let listener = TcpListener::bind(server_address).await.unwrap();
    println!("Listening on address {}", server_address);
    while let Ok((stream, _)) = listener.accept().await {
        println!("Detected potential client");
        let game = game.clone();
        let hub_conn_c = hub_conn.clone();
        tokio::task::spawn(handle_client(stream, game, hub_conn_c));
    }
}

async fn handle_client(stream: TcpStream, game: Arc<RwLock<Game>>, hub_conn: Arc<ServerHubConn>) {
    let mut client_handle = ClientHandle::new(stream, game, hub_conn).await.unwrap();
    loop {
        if let Err(err) = client_handle.update().await {
            eprintln!("{:?}", err);
            return;
        };
    }
}
