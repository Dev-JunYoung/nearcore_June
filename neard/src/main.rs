use chrono::{Utc};
macro_rules! print_file_path_and_function_name {
    () => {
        {
            fn file_write(name:&str) ->std::io::Result<()>{
                //let mut file =std::fs::File::create("mylog.txt").expect("Fail");
                let mut file =std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("mylog.txt")
                    .expect("Fail");
                // Get the current UTC date and time and format it as a string
                //let datetime = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
                // Concatenate the date and time with a string
                //let message = format!("{}",name);
                // Write the concatenated string to the file
                //file.write_all(message.as_bytes())?;
                //std::io::Write::write_all(&mut file, message.as_bytes())?;
                std::io::Write::write_all(&mut file, name.as_bytes())?;
                //file.flush().expect("Fail to flush");
                //std::io::Write::flush(&mut file)?;
                Ok(())
            }
            fn f() {
            }
            fn type_name_of<T>(_: T) -> &'static str {
                std::any::type_name::<T>()
            }
            let name = type_name_of(f);
            let (impl_name, function_name) = match name[..name.len() - 3].rfind("::") {
                Some(pos) => {
                    let impl_end = pos;
                    let impl_start = match name[..impl_end].rfind("::") {
                        Some(pos) => pos + 2,
                        None => 0,
                    };
                    (&name[impl_start..impl_end], &name[impl_end + 2..name.len() - 3])
                }
                None => ("", &name[..name.len() - 3]),
            };
            println!("{}, {:?}, {}, {}, impl: {}", //
               chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),//
                std::thread::current().id(), //
                format!("{}/ fn : {}()", file!(), function_name), //
                line!(), //
                impl_name//
            );

            let message = format!("{}, {:?}, {}, {}, {}, {}", //
               chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),//
                std::thread::current().id(), //
                file!(), function_name, //
                line!(), //
                impl_name);

            file_write(&message);
        }
    };
}
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

mod cli;


use self::cli::NeardCmd;
use anyhow::Context;
use near_primitives::version::{Version, PROTOCOL_VERSION};
use near_store::metadata::DB_VERSION;
use nearcore::get_default_home;
use once_cell::sync::Lazy;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

static NEARD_VERSION: &str = env!("NEARD_VERSION");
static NEARD_BUILD: &str = env!("NEARD_BUILD");
static RUSTC_VERSION: &str = env!("NEARD_RUSTC_VERSION");



fn file_init() -> std::io::Result<()>{
    let mut file =std::fs::File::create("mylog.txt").expect("Fail");
    std::io::Write::write_all(&mut file,"".as_bytes())?;
    Ok(())
}
/*
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
}*/

fn log(name:&str) {
    let formatted=Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    print!("{} ",formatted);
    println!("my-log : {}|| function : {}", file!(),name);
    println!("-------------------------------------------------------------------------------------");
}

static NEARD_VERSION_STRING: Lazy<String> = Lazy::new(|| {
    format!(
        "(release {}) (build {}) (rustc {}) (protocol {}) (db {})",
        NEARD_VERSION, NEARD_BUILD, RUSTC_VERSION, PROTOCOL_VERSION, DB_VERSION
    )
});



fn neard_version() -> Version {
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    log("neard_version()");
    Version {
        version: NEARD_VERSION.to_string(),
        build: NEARD_BUILD.to_string(),
        rustc_version: RUSTC_VERSION.to_string(),
    }

}

// This line defines a lazy static PathBuf variable DEFAULT_HOME, which contains the default path to the NEAR data directory.
// 디렉토리의 기본 경로에 포함하는 지연 정적 PathBuf 변수 를 정의
// what is PathBuf? :  파일 시스템에서 파일 경로를 나타내는 표준 라이브러리
// WHY? 대용량 파일을 다룰때 발생할 수 있는 메모리 문제를 해결
// HOW? 해당 경로만 메모리에 저장되고, 파일은 필요할때, 읽어드림 따라서 효율적으로 관리 가능
static DEFAULT_HOME: Lazy<PathBuf> = Lazy::new(get_default_home);

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
//let timestamp = Utc::now();
//use std::fs::File;
//use std::io::{Write};

fn main() -> anyhow::Result<()> {
    file_init().unwrap();
    print_file_path_and_function_name!();
    //print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    //log("main()");
    //file_write("main()").expect("TODO: panic message");
    if env::var("RUST_BACKTRACE").is_err() {
        // Enable backtraces on panics by default.
        env::set_var("RUST_BACKTRACE", "1");
    }
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    println!("rayon::ThreadPoolBuilder::new()
        .stack_size(8 * 1024 * 1024)
        .build_global()
        .context(\"failed to create the threadpool\")?;");
    //new() :유효한 레이온 스레드 풀 빌더를 생성하고 반환하지만 초기화하지는 않습니다.
    //stack_size() :스택 크기 반환
    //build_global():글로벌 스레드 풀을 초기화합니다.
    // 이 초기화는 선택 사항입니다.
    // 이 함수를 호출하지 않으면 스레드 풀이 기본 구성으로 자동 초기화됩니다.
    // 두 가지 시나리오를 제외하고는 build_global을 호출하지 않는 것이 좋습니다:
    // 기본 구성을 변경하려는 경우.
    // 벤치마크를 실행하는 경우,
    // 첫 번째 반복에서도 워커 스레드가 이미 준비되어 있으므로
    // 초기화하면 약간 더 일관된 결과를 얻을 수 있습니다. 하지만 이 비용은 미미합니다.
    rayon::ThreadPoolBuilder::new()
        .stack_size(8 * 1024 * 1024)
        .build_global()
        .context("failed to create the threadpool")?;
    // We use it to automatically search the for root certificates to perform HTTPS calls
    // (sending telemetry and downloading genesis)
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    println!("openssl_probe::init_ssl_cert_env_vars();");
    //init_ssl_cert_env_vars(): 시스템에서 SSL 인증서를 검색한 다음,
    // 이 프로세스에서 SSL 인증서 SSL_CERT_FILE 및 SSL_CERT_DIR 환경 변수를
    // OpenSSL에서 사용할 수 있도록 구성합니다.
    // 환경 변수가 가리키는 경로가 존재하고 액세스할 수 있는 경우
    // 환경 변수의 사전 구성된 값을 덮어쓰지 않습니다.
    openssl_probe::init_ssl_cert_env_vars();
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    println!("near_performance_metrics::process::schedule_printing_performance_stats(Duration::from_secs(60));");
    near_performance_metrics::process::schedule_printing_performance_stats(Duration::from_secs(60));
    // The default FD soft limit in linux is 1024.
    // We use more than that, for example we support up to 1000 TCP
    // connections, using 5 FDs per each connection.
    // We consider 65535 to be a reasonable limit for this binary,
    // and we enforce it here. We also set the hard limit to the same value
    // to prevent the inner logic from trying to bump it further:
    // FD limit is a global variable, so it shouldn't be modified in an
    // uncoordinated way.
    const FD_LIMIT: u64 = 65535;
    let (_, hard) = rlimit::Resource::NOFILE.get().context("rlimit::Resource::NOFILE::get()")?;
    rlimit::Resource::NOFILE.set(FD_LIMIT, FD_LIMIT).context(format!(
        "couldn't set the file descriptor limit to {FD_LIMIT}, hard limit = {hard}"
    ))?;
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    println!("NeardCmd::parse_and_run()");
    NeardCmd::parse_and_run()
}
