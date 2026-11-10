mod client;
mod event;

pub use client::{WompiClient, WompiClientError, WompiTransaction};
pub use event::{
    VerifiedWompiEvent, WompiEventError, WompiPaymentStatus, reference_for_attempt, verify_event,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WompiEnvironment {
    Test,
    Live,
}

impl WompiEnvironment {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "test" => Ok(Self::Test),
            "live" => Ok(Self::Live),
            _ => Err("wompi_environment_invalid"),
        }
    }

    pub fn wompi_value(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Live => "prod",
        }
    }
}
