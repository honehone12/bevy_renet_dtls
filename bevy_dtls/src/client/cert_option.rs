use rustls::RootCertStore;
use webrtc_dtls::{
    config::{Config, ExtendedMasterSecretType}, 
    crypto::Certificate
};
use crate::cert::loader;

#[derive(Clone)]
pub enum ClientCertOption {
    GenerateSelfSigned {
        subject_alt_name: &'static str
    },
    Load {
        subject_alt_name: &'static str,
        priv_key_path: &'static str,
        certificate_path: &'static str,
        root_ca_path: &'static str
    }
}

impl ClientCertOption {
    pub fn to_dtls_config(self) -> anyhow::Result<Config> {
        let config = match self {
            ClientCertOption::GenerateSelfSigned { 
                subject_alt_name
            } => {
                let cert = Certificate::generate_self_signed(vec![
                    subject_alt_name.to_string()
                ])?;

                Config{
                    certificates: vec![cert],   
                    insecure_skip_verify: true,
                    extended_master_secret: ExtendedMasterSecretType::Require,
                    server_name: subject_alt_name.to_string(),
                    ..Default::default()
                }
            }
            ClientCertOption::Load {
                subject_alt_name, 
                priv_key_path, 
                certificate_path,
                root_ca_path 
            } => {
                let cert = loader::load_key_and_certificate(
                    priv_key_path.into(), 
                    certificate_path.into()
                )?;

                let mut root_ca_store = RootCertStore::empty();
                let root_ca = loader::load_certtificate(root_ca_path.into())?;
                for c in root_ca.iter() {
                    root_ca_store.add(c.clone())?;
                }

                Config{
                    certificates: vec![cert],
                    extended_master_secret: ExtendedMasterSecretType::Require,
                    roots_cas: root_ca_store,
                    server_name: subject_alt_name.to_string(),
                    ..Default::default()
                }
            }
        };

        Ok(config)
    }
}
