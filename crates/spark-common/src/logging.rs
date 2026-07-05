use std::sync::Once;

use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use crate::error::Result;

static INIT_LOGGING: Once = Once::new();

pub fn init_spark_logging(level: Level) -> Result<()> {
    INIT_LOGGING.call_once(|| {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(level)
            .with_writer(std::io::stdout)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    Ok(())
}

pub fn spark_target(name: &str) -> String {
    let normalized = name.trim();
    if normalized.is_empty() {
        "spark".to_string()
    } else if normalized.starts_with("spark.") {
        normalized.to_string()
    } else {
        format!("spark.{normalized}")
    }
}
