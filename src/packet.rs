use std::str::FromStr;

#[allow(dead_code)]
#[derive(Debug)]
pub enum Packet<'a> {
    V1 {
        terminal_no: &'a str,
        time: chrono::NaiveDateTime,
        valid: bool,
        position: (f32, f32),
        speed: f32,
        direction: u16,
        battery: Option<u8>
    },
    Unknown(&'a str)
}

impl<'a> Packet<'a> {
    pub fn from_message(message: &'a str) -> anyhow::Result<Self> {
        if !message.ends_with('#') {
            return Err(anyhow::anyhow!("message must end with '#'"));
        }
        let message = message.trim_end_matches('#');
        let parts: Vec<_> = message.split(",").collect();
        if parts.len() < 3 {
            return Err(anyhow::anyhow!("message must contain at least IHDR, Terminal No., and Operation Name"));
        }

        Ok(match parts.as_slice() {
            ["*HQ", terminal_no, "V1", time, validity, latitude, latitude_symbol, longitude, longitude_symbol, speed, direction, day, .., power] =>
                Self::V1 {
                    terminal_no,
                    time: timestamp(day, time)?,
                    valid: validity == &"A",
                    position: coords(latitude, latitude_symbol, longitude, longitude_symbol)?,
                    speed: nullable::<f32>(speed)? * 1.852,
                    direction: nullable(direction)?,
                    battery: power.parse().ok().and_then(|p| if p > 100 { None } else { Some(p) })
                },
            _ => Self::Unknown(message)
        })
    }
}

fn timestamp(date: &str, time: &str) -> anyhow::Result<chrono::NaiveDateTime> {
    let day = date[0..2].parse()?;
    let month = date[2..4].parse()?;
    let year = date[4..6].parse::<i32>()? + 2000;

    let hours = time[0..2].parse()?;
    let minutes = time[2..4].parse()?;
    let seconds = time[4..6].parse()?;

    Ok(chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or(anyhow::anyhow!("Invalid date"))?.and_hms_opt(hours, minutes, seconds).ok_or(anyhow::anyhow!("Invalid time"))?)
}

fn nullable<T: Default + FromStr>(value: &str) -> anyhow::Result<T> {
    match value {
        "null" => Ok(T::default()),
        _ => value.parse().map_err(|_| anyhow::anyhow!("Invalid nullable value"))
    }
}

fn coords(latitude: &str, latitude_symbol: &str, longitude: &str, longitude_symbol: &str) -> anyhow::Result<(f32, f32)> {
    Ok((coord(latitude, latitude_symbol).ok_or(anyhow::anyhow!("Invalid latitude"))?, coord(longitude, longitude_symbol).ok_or(anyhow::anyhow!("Invalid longitude"))?))
}

fn coord(coord_str: &str, direction: &str) -> Option<f32> {
    let value: f32 = coord_str.parse().ok()?;
    let degrees = (value.floor() / 100.0).floor();
    let minutes = (value - degrees * 100.0) / 60.0;

    let decimal = degrees + minutes;

    Some(match direction {
        "S" | "W" => -decimal,
        _ => decimal
    })
}
