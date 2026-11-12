//! Open-Meteo current weather for the header.

use serde::Deserialize;

use crate::config::Weather;
use crate::format;
use crate::widgets::{Http, Request, WidgetError};

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherNow {
    /// "Berlin, 4°C"
    pub primary: String,
    /// "Cloudy"
    pub condition: &'static str,
    pub icon: Icon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Sun,
    Moon,
    Cloud,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

#[derive(Deserialize)]
struct Forecast {
    current_weather: Current,
    daily: Daily,
}

#[derive(Deserialize)]
struct Current {
    time: String,
    temperature: f64,
    weathercode: u32,
}

#[derive(Deserialize)]
struct Daily {
    sunrise: Vec<String>,
    sunset: Vec<String>,
}

/// WMO weather interpretation codes in words.
fn condition(code: u32, day: bool) -> &'static str {
    match code {
        0 if day => "Sunny",
        0 => "Clear",
        1 if day => "Mainly Sunny",
        1 => "Mainly Clear",
        2 => "Partly Cloudy",
        3 => "Cloudy",
        45 | 48 => "Foggy",
        51 => "Light Drizzle",
        53 => "Drizzle",
        55 => "Heavy Drizzle",
        56 => "Light Freezing Drizzle",
        57 => "Freezing Drizzle",
        61 => "Light Rain",
        63 => "Rain",
        65 => "Heavy Rain",
        66 | 67 => "Freezing Rain",
        71 => "Light Snow",
        73 => "Snow",
        75 => "Heavy Snow",
        77 => "Snow Grains",
        80 => "Light Showers",
        81 => "Showers",
        82 => "Heavy Showers",
        85 | 86 => "Snow Showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm With Hail",
        _ => "Unknown",
    }
}

fn icon(code: u32, day: bool) -> Icon {
    match code {
        0 | 1 if day => Icon::Sun,
        0 | 1 => Icon::Moon,
        2 | 3 => Icon::Cloud,
        45 | 48 => Icon::Fog,
        51..=57 => Icon::Drizzle,
        61..=67 | 80..=82 => Icon::Rain,
        71..=77 | 85 | 86 => Icon::Snow,
        95..=99 => Icon::Thunder,
        _ => Icon::Cloud,
    }
}

pub fn url(cfg: &Weather) -> String {
    let unit = if cfg.units == "metric" {
        "celsius"
    } else {
        "fahrenheit"
    };
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&daily=sunrise,sunset&current_weather=true&temperature_unit={unit}&timezone={}",
        cfg.latitude, cfg.longitude, cfg.timezone
    )
}

pub async fn fetch(http: &dyn Http, cfg: &Weather) -> Result<WeatherNow, WidgetError> {
    let forecast: Forecast = http.send(Request::get(url(cfg))).await?.ok()?.json()?;
    let now = &forecast.current_weather;
    // ISO local timestamps of equal length compare correctly as strings.
    let day = match (
        forecast.daily.sunrise.first(),
        forecast.daily.sunset.first(),
    ) {
        (Some(rise), Some(set)) => now.time > *rise && now.time < *set,
        _ => true,
    };
    let unit = if cfg.units == "metric" { "°C" } else { "°F" };
    let temperature = format::number_max(now.temperature, cfg.maximum_fraction_digits);
    let primary = if cfg.label.is_empty() {
        format!("{temperature}{unit}")
    } else {
        format!("{}, {temperature}{unit}", cfg.label)
    };
    Ok(WeatherNow {
        primary,
        condition: condition(now.weathercode, day),
        icon: icon(now.weathercode, day),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::testing::FakeHttp;
    use crate::widgets::Method;

    const FORECAST: &str = include_str!("../fixtures/openmeteo/forecast.json");

    fn berlin() -> Weather {
        serde_json::from_str(
            r#"{"label":"Berlin","latitude":52.5,"longitude":13.33,"timezone":"Europe/Berlin","maximumFractionDigits":1}"#,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn night_time_cloudy_berlin() {
        let http = FakeHttp::new().on(Method::Get, "api.open-meteo.com/v1/forecast", 200, FORECAST);
        let now = fetch(&http, &berlin()).await.unwrap();
        assert_eq!(now.primary, "Berlin, 4°C");
        assert_eq!(now.condition, "Cloudy");
        assert_eq!(now.icon, Icon::Cloud);
        let url = &http.requests()[0].url;
        assert!(url.contains("latitude=52.5&longitude=13.33"));
        assert!(url.contains("temperature_unit=celsius&timezone=Europe/Berlin"));
    }

    #[test]
    fn day_and_night_wording_differs_for_clear_sky() {
        assert_eq!(condition(0, true), "Sunny");
        assert_eq!(condition(0, false), "Clear");
        assert_eq!(icon(0, false), Icon::Moon);
        assert_eq!(icon(63, true), Icon::Rain);
        assert_eq!(icon(95, true), Icon::Thunder);
    }
}
