use crate::config::Config;
use near_chain_configs::UpdateableClientConfig;
use near_dyn_configs::{UpdateableConfigLoaderError, UpdateableConfigs};
use near_o11y::log_config::LogConfig;
use serde::Deserialize;
use std::path::{Path, PathBuf};

///NEAR 코드베이스용 동적 config 헬퍼.
//
// 이 상자에는 노드가 실행되는 동안 노드를 재구성할 수 있는 유틸리티가 포함되어 있습니다.
//
// 사용 방법:
// 로깅 및 추적
// log_config.json을 변경하고 SIGHUP 신호를 neard 프로세스에 보냅니다.
//
// 기타 구성 값
// config.json을 변경하고 근처 프로세스에 SIGHUP 신호를 보냅니다.
//
// 노드가 실행되는 동안 변경할 수 있는 구성 필드입니다:
// expected_shutdown: 근처드가 정상적으로 종료할 지정된 블록 높이입니다.
// config.json의 다른 필드 변경
// config.json이 유효한 json 객체로 남아 있고 내부 유효성 검사를 통과하는 한 config.json의 다른 필드에 대한 변경 사항은 자동으로 무시됩니다.
//
// 노드가 시작(또는 재시작)될 때 구성 파일의 유효성을 검사하고 문제가 감지되면 충돌이 발생하므로 config.json을 변경할 때 주의하세요.
//

const LOG_CONFIG_FILENAME: &str = "log_config.json";
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
/// This function gets called at the startup and each time a config needs to be reloaded.
pub fn read_updateable_configs(
    home_dir: &Path,
) -> Result<UpdateableConfigs, UpdateableConfigLoaderError> {
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    log("read_updateable_configs()");
    let mut errs = vec![];
    let log_config = match read_log_config(home_dir) {
        Ok(config) => config,
        Err(err) => {
            print!("line: {},{:?} ",std::line!(), std::thread::current().id());
            log("Err()");
            errs.push(err);
            None
        }
    };
    let updateable_client_config =
        match Config::from_file(&home_dir.join(crate::config::CONFIG_FILENAME))
            .map(get_updateable_client_config)
        {
            Ok(config) => Some(config),
            Err(err) => {
                print!("line: {},{:?} ",std::line!(), std::thread::current().id());
                log("Err()");
                errs.push(UpdateableConfigLoaderError::ConfigFileError {
                    file: PathBuf::from(crate::config::CONFIG_FILENAME),
                    err: err.into(),
                });
                None
            }
        };
    if errs.is_empty() {
        crate::metrics::CONFIG_CORRECT.set(1);
        Ok(UpdateableConfigs { log_config, client_config: updateable_client_config })
    } else {
        tracing::warn!(target: "neard", "Dynamically updateable configs are not valid. Please fix this ASAP otherwise the node will be unable to restart: {:?}", &errs);
        crate::metrics::CONFIG_CORRECT.set(0);
        Err(UpdateableConfigLoaderError::Errors(errs))
    }
}
//client config 에 대한 변경가능한 참조를 리턴. 동적 config system의 일부로,
//노드를 다시 시작할 필요 없이 특정 config 매개변수를 업데이트 할 수 있다.
pub fn get_updateable_client_config(config: Config) -> UpdateableClientConfig {
    // All fields that can be updated while the node is running should be explicitly set here.
    // Keep this list in-sync with `core/dyn-configs/README.md`.
    // 노드가 실행되는 동안 업데이트할 수 있는 모든 필드는 여기에 명시적으로 설정해야 합니다.
    // 이 목록을 `core/dyn-configs/README.md`와 동기화 상태로 유지합니다.
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    log("get_updateable_client_config()");
    // UpdateableClientConfig -> 노드가 실행 중일 때 흰색으로 업데이트할 수 있는 구성의 하위 집합입니다
    UpdateableClientConfig { expected_shutdown: config.expected_shutdown }

}

fn read_log_config(home_dir: &Path) -> Result<Option<LogConfig>, UpdateableConfigLoaderError> {
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    log("read_log_config()");
    read_json_config::<LogConfig>(&home_dir.join(LOG_CONFIG_FILENAME))
}

// the file can be JSON with comments
fn read_json_config<T: std::fmt::Debug>(
    path: &Path,
) -> Result<Option<T>, UpdateableConfigLoaderError>
where
    for<'a> T: Deserialize<'a>,
{
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    log("read_json_config()");
    match std::fs::read_to_string(path) {
        Ok(config_str) => match near_config_utils::strip_comments_from_json_str(&config_str) {
            Ok(config_str_without_comments) => {
                match serde_json::from_str::<T>(&config_str_without_comments) {
                    Ok(config) => {
                        tracing::info!(target: "neard", config=?config, "Changing the config {path:?}.");
                        Ok(Some(config))
                    }
                    Err(err) => {
                        Err(UpdateableConfigLoaderError::Parse { file: path.to_path_buf(), err })
                    }
                }
            }
            Err(err) => {
                Err(UpdateableConfigLoaderError::OpenAndRead { file: path.to_path_buf(), err })
            }
        },
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => {
                tracing::info!(target: "neard", ?err, "Reset the config {path:?} because the config file doesn't exist.");
                Ok(None)
            }
            _ => Err(UpdateableConfigLoaderError::OpenAndRead { file: path.to_path_buf(), err }),
        },
    }
}

