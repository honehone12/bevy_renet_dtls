use bevy::prelude::*;
use rustls::crypto::aws_lc_rs;
use super::dtls_server::*;

fn accept_system(mut dtls_server: ResMut<DtlsServer>) {
    let Some(conn_idx) = dtls_server.acpt() else {
        return;
    };

    if let Err(e) = dtls_server.start_conn(conn_idx) {
        panic!("{e}");
    }

    debug!("conn: {} has been started from default system", conn_idx.index());
}

pub struct DtlsServerPlugin {
    pub buf_size: usize,
    pub send_timeout_secs: u64,
    pub recv_timeout_secs: Option<u64>
}

impl Plugin for DtlsServerPlugin {
    fn build(&self, app: &mut App) {
        if aws_lc_rs::default_provider()
        .install_default()
        .is_err() {
            panic!("failed to setup crypto provider");
        }

        let dtls_server = match DtlsServer::new(
            self.buf_size, 
            self.send_timeout_secs,
            self.recv_timeout_secs
        ) {
            Ok(s) => s,
            Err(e) => panic!("{e}")
        };

        app.insert_resource(dtls_server)
        .add_systems(PreUpdate, accept_system);
    }
}
