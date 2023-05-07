use near_chain_configs::UpdateableClientConfig;
use near_dyn_configs::{UpdateableConfigLoaderError, UpdateableConfigs};
use std::sync::Arc;
use tokio::sync::broadcast::Receiver;


use chrono::{Utc};
fn file_write(name:&str) ->std::io::Result<()>{
    let mut file =std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("mylog.txt")
        .expect("Fail");
    // Get the current UTC date and time and format it as a string
    let datetime = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    // Concatenate the date and time with a string
    let message = format!("{} {:?} {} {} \n", datetime,std::thread::current().id(),file!(),name);
    // Write the concatenated string to the file
    std::io::Write::write_all(&mut file, message.as_bytes())?;
    Ok(())
}

fn log(name:&str) {
    let formatted=Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    print!("{} ",formatted);
    println!("my-log : {}|| function : {}", file!(),name);
    println!("-------------------------------------------------------------------------------------");
}
#[derive(Debug)]
pub enum ClientConfigUpdateError {}

/// Manages updating the config encapsulating.
/// 설정 캡슐화 업데이트를 관리합니다.
pub struct ConfigUpdater {
    /// Receives config updates while the node is running.
    /// 노드가 실행되는 동안 설정 업데이트를 수신합니다.
    rx_config_update: Receiver<Result<UpdateableConfigs, Arc<UpdateableConfigLoaderError>>>,

    /// Represents the latest Error of reading the dynamically reloadable configs.
    /// 동적으로 다시 로드할 수 있는 설정의 최신 오류를 나타냅니다.
    updateable_configs_error: Option<Arc<UpdateableConfigLoaderError>>,
}

impl ConfigUpdater {
    pub fn new(
        rx_config_update: Receiver<Result<UpdateableConfigs, Arc<UpdateableConfigLoaderError>>>,
    ) -> Self {
        log("new() / in impl ConfigUpdater");
        Self { rx_config_update, updateable_configs_error: None }
    }

    /// Check if any of the configs were updated.
    /// If they did, the receiver (rx_config_update) will contain a clone of the new configs.
    /// 설정이 업데이트되었는지 확인합니다.
    /// 업데이트된 설정이 있다면 수신자(rx_config_update)에 새 설정의 복제본이 포함됩니다.
    pub fn try_update(&mut self, update_client_config_fn: &dyn Fn(UpdateableClientConfig)) {
        while let Ok(maybe_updateable_configs) = self.rx_config_update.try_recv() {
            print!("line: {},{:?} ",std::line!(), std::thread::current().id());
            log("try_update()");
            match maybe_updateable_configs {
                Ok(updateable_configs) => {
                    if let Some(client_config) = updateable_configs.client_config {
                        update_client_config_fn(client_config);
                        print!("line: {},{:?} ",std::line!(), std::thread::current().id());
                        log("try_update() - match maybe_updateable_configs");
                        tracing::info!(target: "config", "Updated ClientConfig");
                    }
                    self.updateable_configs_error = None;
                }
                Err(err) => {
                    self.updateable_configs_error = Some(err.clone());
                }
            }
        }
    }

    /// Prints an error if it's present.
    pub fn report_status(&self) {
        log("report_status() / in impl ConfigUpdater");
        if let Some(updateable_configs_error) = &self.updateable_configs_error {
            tracing::warn!(
                target: "stats",
                "Dynamically updateable configs are not valid. Please fix this ASAP otherwise the node will probably crash after restart: {}",
                *updateable_configs_error);
        }
    }
}
