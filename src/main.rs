mod packet;

use log::info;
use sqlx::{mysql::MySqlPoolOptions, Pool};
use std::net::SocketAddr;
use std::time::Duration;
use packet::Packet;
use sqlx::MySql;
use tokio::io::AsyncWriteExt;
use tokio::time::{interval, timeout};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{TcpListener, TcpStream},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let socket_port = std::env::var("PORT")
        .expect("Missing PORT environment variable")
        .parse::<u16>()?;
    let dsn = std::env::var("DATABASE_URL").expect("Missing DATABASE_URL environment variable");

    let pool = MySqlPoolOptions::new()
        .max_connections(10)
        .connect(&dsn)
        .await?;

    let addr: SocketAddr = ([0, 0, 0, 0], socket_port).into();
    info!("Listening on port {socket_port}, with db={dsn}");

    let listener = TcpListener::bind(addr).await?;
    loop {
        let (stream, addr) = listener.accept().await?;
        let pool = pool.clone();
        tokio::spawn(async move {
            info!("Accepted a connection: {addr}");
            match handle_connection(stream, pool).await {
                Ok(_) => info!("Connection with {addr} ended"),
                Err(e) => info!("{addr} dropped, {}", e),
            }
        });
    }
}

async fn handle_connection(stream: TcpStream, pool: Pool<MySql>) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);

    loop {
        let mut buffer = Vec::new();

        let read = timeout(Duration::from_secs(240), reader.read_until(b'#', &mut buffer)).await??;
        let message = &buffer[..read];
        let Ok(message) = String::from_utf8(message.to_vec()) else {
            continue;
        };

        info!("Message received: {:?}", &message);
        let message = Packet::from_message(&message)?;
        match message {
            Packet::V1 { terminal_no, time, position, speed, direction, battery, .. } => {
                let obtained_before: Option<(i64, f32, f32)> = sqlx::query_as("select id, lat, lon from locations where device_id = ? and obtained_at < ? order by obtained_at desc limit 1")
                    .bind(terminal_no)
                    .bind(time)
                    .fetch_optional(&pool)
                    .await?;
                if let Some((id, lat, lon)) = obtained_before {
                    if lat == position.0 && lon == position.1 {
                        sqlx::query("DELETE FROM locations WHERE id = ?")
                            .bind(id)
                            .execute(&pool)
                            .await?;
                    }
                }

                sqlx::query("insert into locations(device_id, obtained_at, lat, lon, speed, direction, battery) values (?, ?, ?, ?, ?, ?, ?)")
                    .bind(terminal_no)
                    .bind(time)
                    .bind(position.0)
                    .bind(position.1)
                    .bind(speed)
                    .bind(direction)
                    .bind(battery)
                    .execute(&pool)
                    .await?;
            }
            Packet::Unknown(_) => info!("Skipping unknown packet"),
        }
    }
}
