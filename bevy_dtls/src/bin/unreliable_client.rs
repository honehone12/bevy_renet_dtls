use std::net::{IpAddr, Ipv4Addr};
use bevy::{
    log::{Level, LogPlugin}, 
    prelude::*
};
use bytes::Bytes;
use bevy_dtls::client::{
    cert_option::ClientCertOption, dtls_client::*, plugin::DtlsClientPlugin
};

#[derive(Component)]
struct RollingBox;

fn setup_graphics(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>
) {
    commands.spawn(DirectionalLightBundle{
        transform: Transform{
            translation: Vec3::new(0.0, 10.0, 0.0),
            rotation: Quat::from_rotation_x(-std::f32::consts::PI / 2.0),
            ..default()
        },
        ..default()
    });

    commands.spawn(Camera3dBundle{
        transform: Transform::from_translation(Vec3::new(0.0, 10.0, 10.0))
        .looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    commands.spawn(PbrBundle{
        mesh: meshes.add(Mesh::from(Cuboid::from_size(Vec3::new(3.0, 3.0, 3.0)))),
        material: materials.add(Color::from(bevy::color::palettes::basic::MAROON)),
        ..default()
    })
    .insert(RollingBox);
}

fn graphics_system(
    mut query: Query<&mut Transform, With<RollingBox>>,
    time: Res<Time>
) {
    for mut transform in query.iter_mut() {
        transform.rotate_y(std::f32::consts::PI * 0.5 * time.delta_seconds());
    }
}

struct ClientGraphicsPlugin;

impl Plugin for ClientGraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_graphics)
        .add_systems(Update, graphics_system);
    }
}

#[derive(Resource)]
struct ClientHellooonCounter(usize);

fn send_hellooon_system(
    dtls_client: Res<DtlsClient>, 
    mut counter: ResMut<ClientHellooonCounter>
) {
    let str = format!("from client helloooooon {}", counter.0);
    let msg = Bytes::from(str);
    match dtls_client.send(msg) {
        Ok(_) => counter.0 += 1, 
        Err(e) => error!("{e}")
    }
}

fn recv_hellooon_system(mut dtls_client: ResMut<DtlsClient>) {
    loop {
        let Some(bytes) = dtls_client.recv() else {
            return;
        };

        let msg = String::from_utf8(bytes.to_vec())
        .unwrap();
        info!("message: {msg}");
    }
}

fn timeout_check_system(mut dtls_client: ResMut<DtlsClient>) {
    loop {
        let Err(_) = dtls_client.timeout_check() else {
            return;
        };

        error!("sending timeout, but still available to re-try");
    }
}

fn health_check_system(mut dtls_client: ResMut<DtlsClient>) {
    let health = dtls_client.health_check();
    if let Some(Err(e)) = health.sender {
        panic!("sender: {e}");
    }
    if let Some(Err(e)) = health.recver {
        panic!("recver: {e}");
    }
}

struct ClientPlugin {
    server_addr: IpAddr,
    server_port: u16,
    client_addr: IpAddr,
    client_port: u16,
    cert_option: ClientCertOption
}

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        let mut dtls_client = app.world_mut()
        .resource_mut::<DtlsClient>();
    
        if let Err(e) = dtls_client.start(DtlsClientConfig{ 
            server_addr: self.server_addr, 
            server_port: self.server_port,
            client_addr: self.client_addr, 
            client_port: self.client_port,
            cert_option: self.cert_option.clone()
        }) {
            panic!("{e}")
        }
    }
}

fn main() {
    App::new()
    .add_plugins((
        DefaultPlugins.set(LogPlugin{
            level: Level::INFO,
            ..default()
        }),
        DtlsClientPlugin{
            buf_size: 512,
            timeout_secs: 10
        }
    ))
    .add_plugins((
        ClientGraphicsPlugin,
        ClientPlugin{
            server_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            server_port: 4443,
            client_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            client_port: 0,
            cert_option: ClientCertOption::GenerateSelfSigned { 
                subject_alt_name: "webrtc.rs" 
            }
            // cert_option: ClientCertOption::Load { 
            //     subject_alt_name: "webrtc.rs", 
            //     priv_key_path: "my_certificates/client.priv.pem", 
            //     certificate_path: "my_certificates/client.pub.pem",
            //     root_ca_path: "my_certificates/server.pub.pem" 
            // }
        }
    ))
    .insert_resource(ClientHellooonCounter(0))
    .add_systems(Update, (
        recv_hellooon_system,
        send_hellooon_system,
        timeout_check_system,
        health_check_system
    ))
    .run();
}
