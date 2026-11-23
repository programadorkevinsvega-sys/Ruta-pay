use std::{env, error::Error, io, net::IpAddr};
use url::Url;

pub type DependencyError = Box<dyn Error + Send + Sync>;

pub fn is_remote_host(url: &Url) -> Result<bool, io::Error> {
    let host = url
        .host_str()
        .ok_or_else(|| configuration_error("test dependency URL must include a host"))?;
    Ok(!host.eq_ignore_ascii_case("localhost")
        && !host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()))
}

pub fn mtls_required() -> Result<bool, io::Error> {
    match env::var("PAYVIA_REQUIRE_MTLS") {
        Ok(value) if value.trim().eq_ignore_ascii_case("true") => Ok(true),
        Ok(value) if value.trim().eq_ignore_ascii_case("false") => Ok(false),
        Ok(_) => Err(configuration_error(
            "PAYVIA_REQUIRE_MTLS must be true or false",
        )),
        Err(_) => Ok(false),
    }
}

pub fn ensure_remote_test_environment(remote: bool) -> Result<(), io::Error> {
    let environment = env::var("APP_ENV").unwrap_or_else(|_| "development".to_owned());
    let require_mtls = mtls_required()?;
    if matches!(environment.as_str(), "staging" | "production") && !require_mtls {
        return Err(configuration_error(
            "PAYVIA_REQUIRE_MTLS=true is mandatory in staging and production",
        ));
    }
    if remote
        && (environment == "production"
            || !matches!(environment.as_str(), "development" | "staging"))
    {
        return Err(configuration_error(
            "remote acceptance requires APP_ENV=development or staging",
        ));
    }
    if remote && environment == "staging" && !require_mtls {
        return Err(configuration_error(
            "remote staging acceptance requires PAYVIA_REQUIRE_MTLS=true",
        ));
    }
    Ok(())
}

pub fn configuration_error(message: &str) -> io::Error {
    io::Error::other(message.to_owned())
}
