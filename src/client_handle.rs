use super::*;
use futures::StreamExt;
use std::result::Result;
use stellar_bit_core::network::NetworkError;
use tokio_tungstenite::WebSocketStream;

pub struct ClientHandle {
    ws_stream: WebSocketStream<TcpStream>,
    game: Arc<RwLock<Game>>,
    user: User,
    last_cmd_index: usize,
    msg_count: usize,
    hub_conn: Arc<ServerHubConn>
}

impl ClientHandle {
    pub async fn new(
        stream: TcpStream,
        game: Arc<RwLock<Game>>,
        hub_conn: Arc<ServerHubConn>
    ) -> Result<Self, network::NetworkError> {
        let ws_stream = accept_async(stream)
            .await
            .map_err(|_| NetworkError::WebsocketTrouble)?;

        Ok(Self {
            ws_stream,
            game,
            user: User::Spectator,
            last_cmd_index: 0,
            msg_count: 0,
            hub_conn
        })
    }

    pub async fn update(&mut self) -> Result<(), NetworkError> {
        let Some(client_msg_result) = self.receive_msg().await else {
            return Err(NetworkError::NoMsgReceived);
        };

        println!(
            "Client registered as user: {:?} has sent his {}th message",
            self.user, self.msg_count
        );
        self.msg_count += 1;

        let client_msg = client_msg_result?;
        let response = self.handle_msg(client_msg).await?;
        self.send_response(response).await?;

        Ok(())
    }

    pub async fn handle_msg(&mut self, msg: ClientRequest) -> Result<ServerResponse, NetworkError> {
        match msg {
            ClientRequest::Join(public_token, access_token) => {
                self.user = User::Spectator;

                if !self.hub_conn.verify(public_token as i64, access_token).await {
                    return Err(NetworkError::WrongAuthToken);
                }
                let mut game = self.game.write().unwrap();

                if !game.players.contains_key(&public_token) {
                    game.execute_cmd(User::Server, GameCmd::AddPlayer(public_token))
                        .unwrap();
                    game.execute_cmd(
                        User::Server,
                        GameCmd::SpawnStarBase(
                            public_token,
                            Vec2::random_unit_circle() * 1000.,
                            vec2(0., 0.),
                        ),
                    )
                    .unwrap();
                    game.execute_cmd(
                        User::Server,
                        GameCmd::GiveMaterials(
                            public_token,
                            vec![
                                (Material::Iron, 10_000.),
                                (Material::Nickel, 10_000.),
                                (Material::Silicates, 10_000.),
                                (Material::Copper, 10_000.),
                                (Material::Carbon, 10_000.),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    )
                    .unwrap();
                }
                self.user = User::Player(public_token);
                Ok(ServerResponse::SetUser(self.user))
            }
            ClientRequest::ExecuteGameCmds(new_game_cmds) => {
                for cmd in new_game_cmds {
                    if let Err(err) = self
                        .game
                        .write()
                        .unwrap()
                        .execute_cmd(self.user, cmd.clone())
                    {
                        println!(
                            "Error executing command: {:?}, by user: {:?}",
                            err, self.user
                        );
                    }
                }

                Ok(ServerResponse::Success)
            }
            ClientRequest::FullGameSync => {
                let game = self.game.read().unwrap().clone();
                self.last_cmd_index = game.cmds_history.len();
                Ok(ServerResponse::SyncFullGame(game))
            }
            ClientRequest::GameCmdsSync => {
                let game = self.game.read().unwrap();
                let mut game_cmds = vec![];
                if self.last_cmd_index < game.cmds_history.len() {
                    game_cmds = game.cmds_history[self.last_cmd_index..]
                        .iter()
                        .map(|x| (x.user, x.cmd.clone()))
                        .collect();
                }
                self.last_cmd_index = game.cmds_history.len();
                Ok(ServerResponse::SyncGameCmds(game_cmds))
            }
            ClientRequest::SyncClock => Ok(ServerResponse::SyncClock(now())),
        }
    }

    pub async fn send_response(&mut self, msg: ServerResponse) -> Result<(), NetworkError> {
        let msg_raw = serialize_bytes(&msg).unwrap();

        self.ws_stream
            .feed(Message::Binary(msg_raw))
            .await
            .map_err(|_| NetworkError::WebsocketTrouble)?;
        self.ws_stream
            .flush()
            .await
            .map_err(|_| NetworkError::WebsocketTrouble)
    }

    pub async fn receive_msg(&mut self) -> Option<Result<ClientRequest, NetworkError>> {
        self.ws_stream
            .next()
            .await
            .map(|msg_result| match msg_result {
                Ok(msg) => {
                    if let Message::Binary(client_msg_raw) = msg {
                        deserialize_bytes(&client_msg_raw)
                            .map_err(|_| NetworkError::IncorrectDataFormat)
                    } else {
                        Err(NetworkError::IncorrectDataFormat)
                    }
                }
                Err(_) => Err(NetworkError::WebsocketTrouble),
            })
    }
}
