mod packet;

use log::info;
use sqlx::{mysql::MySqlPoolOptions, Pool};
use std::net::SocketAddr;
use std::time::Duration;
use packet::Packet;
use sqlx::MySql;
use tokio::time::timeout;
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
            Packet::V1 { terminal_no, time, position, battery, .. } => {
                let before = sqlx::query!("select id, ST_Equals(location, ST_GeomFromText(?)) as same_location from readings where device_id = ? and obtained_at < ? order by obtained_at desc limit 1", format!("POINT({} {})", position.1, position.0), terminal_no, time)
                    .fetch_optional(&pool)
                    .await?;

                let after = sqlx::query!("select id, ST_Equals(location, ST_GeomFromText(?)) as same_location from readings where device_id = ? and obtained_at > ? order by obtained_at desc limit 1", format!("POINT({} {})", position.1, position.0), terminal_no, time)
                    .fetch_optional(&pool)
                    .await?;

                let query = match (before, after) {
                    (Some(before), Some(after)) if boolean(before.same_location) && boolean(after.same_location) => {
                        info!("merging three consecutive locations");
                        // delete the older one, as the duplicates are not needed (same location in contiguous sequence)
                        Some(sqlx::query!("delete from readings where id = ?", before.id))
                        // don't insert the new one, as there is a newer record
                    }
                    (Some(before), _) if boolean(before.same_location) => {
                        info!("updating before location");
                        Some(sqlx::query!("update readings set obtained_at = ?, battery = ? where id = ?", time, battery, before.id))
                    }
                    (_, Some(after)) if boolean(after.same_location) => {
                        info!("already a newer record present, doing nothing");
                        None
                    }
                    _ => {
                        info!("inserting new record");
                        Some(sqlx::query!("insert into readings(device_id, obtained_at, location, battery) values (?, ?, ST_GeomFromText(?), ?)", terminal_no, time, format!("POINT({} {})", position.1, position.0), battery))
                    }
                };

                if let Some(query) = query {
                    if let Err(e) = query.execute(&pool).await {
                        info!("Error executing query: {}", e);
                    }
                }
            }
            Packet::Unknown(_) => info!("Skipping unknown packet"),
        }
    }
}

fn boolean(v: Option<i32>) -> bool {
    v.and_then(|v| Some(v != 0)).unwrap_or_default()
}