use tokio::sync::watch;

pub type ConfigContext = watch::Receiver<ConfigContextData>;

pub struct ConfigContextData {}
