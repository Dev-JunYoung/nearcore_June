use crate::stats::print_performance_stats;
use std::thread;
use std::time::Duration;
use tracing::{error, info};
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

// 함수는 지정된 간격으로 NEAR 프로토콜 노드 성능 통계 인쇄를 예약하고
// 절전 시간이 0인 경우 성능 통계 인쇄를 비활성화합니다.
pub fn schedule_printing_performance_stats(sleep_time: Duration) {
    file_write("schedule_printing_performance_stats").unwrap();
    print!("line: {},{:?} ",std::line!(), std::thread::current().id());
    log("schedule_printing_performance_stats()");
    if cfg!(feature = "performance_stats") {
        if sleep_time.is_zero() {
            info!("print_performance_stats: disabled");
            return;
        }
        info!("print_performance_stats: enabled");

        if let Err(err) =
            thread::Builder::new().name("PerformanceMetrics".to_string()).spawn(move || loop {
                print_performance_stats(sleep_time);
                thread::sleep(sleep_time);
            })
        {
            error!("failed to spawn the thread: {}", err);
        }
    }
}
