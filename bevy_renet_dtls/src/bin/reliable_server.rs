use std::{
    net::{IpAddr, Ipv4Addr}, 
    time::Duration
};
use bevy::{
    app::ScheduleRunnerPlugin, 
    log::{Level, LogPlugin}, 
    prelude::*
};
use bevy_renet::renet::{ConnectionConfig, RenetServer};
use bevy_dtls::server::{
    cert_option::ServerCertOption, 
    dtls_server::{DtlsServer, DtlsServerConfig}
};
use bevy_renet_dtls::server::renet_dtls_server::RenetDtlsServerPlugin;

struct ServerPlugin {
    listen_addr: IpAddr,
    listen_port: u16,
    cert_option: ServerCertOption
}

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        let renet_server = RenetServer::new(ConnectionConfig::default());
        app.insert_resource(renet_server);

        let mut dtls_server = app.world_mut()
        .resource_mut::<DtlsServer>();
        if let Err(e) = dtls_server.start(DtlsServerConfig{
            listen_addr: self.listen_addr,
            listen_port: self.listen_port,
            cert_option: self.cert_option
        }) {
            panic!("{e}");
        }

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
        RenetDtlsServerPlugin{
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
    .run();
}
