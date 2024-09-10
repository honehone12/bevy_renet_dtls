use std::{
    net::{IpAddr, Ipv4Addr}, 
    time::Duration
};
use bevy::{
    app::ScheduleRunnerPlugin, 
    log::{Level, LogPlugin}, 
    prelude::*
};
use bevy_renet::{renet::{ConnectionConfig, DefaultChannel, RenetServer}, RenetServerPlugin};
use bevy_dtls::server::{
    cert_option::ServerCertOption, 
    dtls_server::{DtlsServer, DtlsServerConfig}
};
use bevy_renet_dtls::server::renet_dtls_server::RenetDtlsServerPlugin;
use bytes::Bytes;

#[derive(Resource)]
struct ServerHellooonCounter(usize);

fn send_hellooon_system(
    mut renet_server: ResMut<RenetServer>,
    mut counter: ResMut<ServerHellooonCounter>
) {
    if renet_server.connected_clients() == 0 {
        return;
    }

    let str = format!("from server helloooooon {}", counter.0);
    let msg = Bytes::from(str);
    renet_server.broadcast_message(DefaultChannel::ReliableOrdered, msg);
    counter.0 += 1;
}

fn recv_hellooon_system(mut renet_server: ResMut<RenetServer>) {
    let ch_len = 3_u8;
    let clients = renet_server.clients_id();

    for client_id in clients {
        for ch in 0..ch_len {
            loop {
                let Some(bytes) = renet_server.receive_message(client_id, ch) else {
                    break;
                };
    
                let msg = String::from_utf8(bytes.to_vec()).unwrap();
                info!("message from: {client_id}: {msg}");
            }
        }
    }    
}

struct ServerPlugin {
    listen_addr: IpAddr,
    listen_port: u16,
    cert_option: ServerCertOption
}

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        let mut dtls_server = app.world_mut()
        .resource_mut::<DtlsServer>();
        if let Err(e) = dtls_server.start(DtlsServerConfig{
            listen_addr: self.listen_addr,
            listen_port: self.listen_port,
            cert_option: self.cert_option
        }) {
            panic!("{e}");
        }

        let renet_server = RenetServer::new(ConnectionConfig::default());
        app.insert_resource(renet_server);

        info!(
            "server is listening at {}:{}", 
            self.listen_addr,
            self.listen_port
        );
    }
}

fn main() {
    App::new()
    .add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f32(1.0 / 30.0)
        )),
        LogPlugin{
            level: Level::INFO,
            ..default()
        },
        RenetServerPlugin,
        RenetDtlsServerPlugin{
            max_clients: 1,
            buf_size: 512,
            send_timeout_secs: 10,
            recv_timeout_secs: None
        }
    ))
    .add_plugins(ServerPlugin{
        listen_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
        listen_port: 4443,
        cert_option: ServerCertOption::GenerateSelfSigned { 
            subject_alt_name: "webrtc.rs"
        }
    })
    .insert_resource(ServerHellooonCounter(0))
    .add_systems(Update, (
        recv_hellooon_system,
        send_hellooon_system
    ))
    .run();
}
