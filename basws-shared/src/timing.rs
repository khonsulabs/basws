use chrono::Utc;

pub fn current_timestamp() -> f64 {
    Utc::now().timestamp_nanos() as f64 / 1_000_000_000.0
}

#[derive(Default)]
pub struct NetworkTiming {
    pub average_roundtrip: Option<f64>,
    pub average_server_timestamp_delta: Option<f64>,
}

impl NetworkTiming {
    pub fn update(&mut self, original_timestamp: f64, timestamp: f64) {
        let now = current_timestamp();
        let roundtrip = now - original_timestamp;

        self.average_roundtrip = Some(match self.average_roundtrip {
            Some(average_roundtrip) => (average_roundtrip * 4.0 + roundtrip) / 5.0,
            None => roundtrip,
        });

        let timestamp_delta = (now - timestamp) - roundtrip / 2.0;
        self.average_server_timestamp_delta = Some(match self.average_server_timestamp_delta {
            Some(average_server_timestamp_delta) => {
                (average_server_timestamp_delta * 4.0 + timestamp_delta) / 5.0
            }
            None => timestamp_delta,
        });
    }
}
