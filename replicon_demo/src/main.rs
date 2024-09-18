use std::{error::Error, net::{IpAddr, Ipv4Addr}};
use bevy::{
    color::palettes::css::GREEN, prelude::*
};
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{
    renet::{ConnectionConfig, RenetServer, RenetClient}, 
    RenetChannelsExt, RepliconRenetPlugins
};
use bevy_renet_dtls::{
    client::{RenetClientDtlsExt, RenetDtlsClientPlugin}, dtls::{
        client::{
            cert_option::ClientCertOption, dtls_client::{DtlsClient, DtlsClientConfig}, health::DtlsClientError
        }, server::{
            cert_option::ServerCertOption, dtls_server::{DtlsServer, DtlsServerConfig}, health::DtlsServerError
        }
    }, server::RenetDtlsServerPlugin
};
use serde::{Serialize, Deserialize};
use clap::Parser;

const PORT: u16 = 4443;

#[derive(Parser, PartialEq, Resource)]
enum Cli {
    SinglePlayer,
    Server {
        #[arg(short, long, default_value_t = PORT)]
        port: u16
    },
    Client {
        #[arg(short, long, default_value_t = Ipv4Addr::LOCALHOST.into())]
        ip: IpAddr,
        #[arg(short, long, default_value_t = PORT)]
        port: u16
    }
}

impl Default for Cli {
    fn default() -> Self {
        Self::parse()
    }
}

#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    position: PlayerPosition,
    color: PlayerColor,
    replicated: Replicated,
}

impl PlayerBundle {
    fn new(client_id: ClientId, position: Vec2, color: Color) -> Self {
        Self {
            player: Player(client_id),
            position: PlayerPosition(position),
            color: PlayerColor(color),
            replicated: Replicated,
        }
    }
}

#[derive(Component, Serialize, Deserialize)]
struct Player(ClientId);

#[derive(Component, Serialize, Deserialize, Deref, DerefMut)]
struct PlayerPosition(Vec2);

#[derive(Component, Serialize, Deserialize)]
struct PlayerColor(Color);

#[derive(Event, Serialize, Deserialize, Debug, Default)]
struct MoveDirection(Vec2);

fn read_cli(
    mut commands: Commands,
    cli: Res<Cli>,
    channels: Res<RepliconChannels>,
    mut server_transport: ResMut<DtlsServer>,
    mut client_transport: ResMut<DtlsClient>,
) -> Result<(), Box<dyn Error>> {
    match *cli {
        Cli::SinglePlayer => {
            commands.spawn(PlayerBundle::new(
                ClientId::SERVER,
                Vec2::ZERO,
                GREEN.into(),
            ));
        }
        Cli::Server { port } => {
            let server_channels_config = channels.get_server_configs();
            let client_channels_config = channels.get_client_configs();

            let server = RenetServer::new(ConnectionConfig {
                server_channels_config,
                client_channels_config,
                ..Default::default()
            });
            
            server_transport.start(DtlsServerConfig{
                listen_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                listen_port: port,
                cert_option: ServerCertOption::LoadWithClientAuth { 
                    priv_key_path: "my_certificates/server.priv.pem", 
                    certificate_path: "my_certificates/server.pub.pem",
                    client_ca_path: "my_certificates/server.pub.pem" 
                }
            })?;

            commands.insert_resource(server);
            commands.spawn(TextBundle::from_section(
                "Server",
                TextStyle {
                    font_size: 30.0,
                    color: Color::WHITE,
                    ..default()
                },
            ));
            commands.spawn(PlayerBundle::new(
                ClientId::SERVER,
                Vec2::ZERO,
                GREEN.into(),
            ));
        }
        Cli::Client { port, ip } => {
            let server_channels_config = channels.get_server_configs();
            let client_channels_config = channels.get_client_configs();

            let mut client = RenetClient::new(ConnectionConfig {
                server_channels_config,
                client_channels_config,
                ..Default::default()
            });
            
            client.start_with_dtls(
                &mut client_transport,
                DtlsClientConfig{
                    server_addr: ip,
                    server_port: port,
                    client_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    client_port: 0,
                    cert_option: ClientCertOption::LoadWithClientAuth { 
                        server_name: "webrtc.rs", 
                        priv_key_path: "my_certificates/client.priv.pem", 
                        certificate_path: "my_certificates/client.pub.pem",
                        root_ca_path: "my_certificates/server.pub.pem" 
                    }
                }
            )?;

            commands.insert_resource(client);
            commands.spawn(TextBundle::from_section(
                format!("Client"),
                TextStyle {
                    font_size: 30.0,
                    color: Color::WHITE,
                    ..default()
                },
            ));
        }
    }

    Ok(())
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2dBundle::default());
}

fn handle_connections(mut commands: Commands, mut server_events: EventReader<ServerEvent>) {
    for event in server_events.read() {
        match event {
            ServerEvent::ClientConnected { client_id } => {
                info!("{client_id:?} connected");
                let r = ((client_id.get() % 23) as f32) / 23.0;
                let g = ((client_id.get() % 27) as f32) / 27.0;
                let b = ((client_id.get() % 39) as f32) / 39.0;
                commands.spawn(PlayerBundle::new(
                    *client_id,
                    Vec2::ZERO,
                    Color::srgb(r, g, b),
                ));
            }
            ServerEvent::ClientDisconnected { client_id, reason } => {
                info!("{client_id:?} disconnected: {reason}");
            }
        }
    }
}

fn read_input(mut move_events: EventWriter<MoveDirection>, input: Res<ButtonInput<KeyCode>>) {
    let mut direction = Vec2::ZERO;
    if input.pressed(KeyCode::ArrowRight) {
        direction.x += 1.0;
    }
    if input.pressed(KeyCode::ArrowLeft) {
        direction.x -= 1.0;
    }
    if input.pressed(KeyCode::ArrowUp) {
        direction.y += 1.0;
    }
    if input.pressed(KeyCode::ArrowDown) {
        direction.y -= 1.0;
    }
    if direction != Vec2::ZERO {
        move_events.send(MoveDirection(direction.normalize_or_zero()));
    }
}

fn apply_movement(
    time: Res<Time>,
    mut move_events: EventReader<FromClient<MoveDirection>>,
    mut players: Query<(&Player, &mut PlayerPosition)>,
) {
    const MOVE_SPEED: f32 = 300.0;
    for FromClient { client_id, event } in move_events.read() {
        info!("received event {event:?} from {client_id:?}");
        for (player, mut position) in &mut players {
            if *client_id == player.0 {
                **position += event.0 * time.delta_seconds() * MOVE_SPEED;
            }
        }
    }
}

fn draw_boxes(mut gizmos: Gizmos, players: Query<(&PlayerPosition, &PlayerColor)>) {
    for (position, color) in &players {
        gizmos.rect(
            Vec3::new(position.x, position.y, 0.0),
            Quat::IDENTITY,
            Vec2::ONE * 50.0,
            color.0,
        );
    }
}

fn handle_error(
    mut server_errors: EventReader<DtlsServerError>,
    mut client_errors: EventReader<DtlsClientError>
) {
    for e in server_errors.read() {
        error!("{e:?}");
    }

    for e in client_errors.read() {
        error!("{e:?}");
    }
}

struct SimpleBoxPlugin;

impl Plugin for SimpleBoxPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<PlayerPosition>()
            .replicate::<PlayerColor>()
            .add_client_event::<MoveDirection>(ChannelKind::Ordered)
            .add_systems(
                Startup,
                (read_cli.map(Result::unwrap), spawn_camera),
            )
            .add_systems(
                Update,
                (
                    apply_movement.run_if(has_authority), 
                    handle_connections.run_if(server_running), 
                    (draw_boxes, read_input, handle_error),
                ),
            );
    }
}

fn main() {
    App::new()
    .init_resource::<Cli>()
    .add_plugins((
        DefaultPlugins,
        RepliconPlugins,
        RepliconRenetPlugins,
        RenetDtlsServerPlugin{
            max_clients: 10,
            buf_size: 1500,
            send_timeout_secs: 10,
            recv_timeout_secs: None,
        },
        RenetDtlsClientPlugin{
            timeout_secs: 10,
            buf_size: 1500,
        },
        SimpleBoxPlugin,
    ))
    .run();
}