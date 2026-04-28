use std::time::Duration;

pub async fn get_weather(location: &str) -> String {
    trpl::sleep(Duration::from_secs(2)).await;
    format!("The weather in {location} is sunny with a high of 25°C.")
}

