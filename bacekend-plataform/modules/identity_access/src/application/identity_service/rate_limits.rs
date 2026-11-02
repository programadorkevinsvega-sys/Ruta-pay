use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use redis::AsyncCommands;
use sha2::{Digest, Sha256};

use super::{IdentityError, IdentityService};

const LOGIN_WINDOW_SECONDS: u64 = 60;
const LOGIN_WINDOW_LIMIT: u64 = 10;

impl IdentityService {
    pub(super) async fn enforce_login_rate_limit(
        &self,
        email: &str,
        source_ip: &str,
    ) -> Result<(), IdentityError> {
        let source_ip = source_ip
            .parse::<std::net::IpAddr>()
            .map_err(|_| IdentityError::SourceAddressInvalid)?;
        let account_digest = Sha256::digest(email.as_bytes());
        let ip_digest = Sha256::digest(source_ip.to_string().as_bytes());
        let account_key = format!(
            "platform:identity:login:account:{}",
            URL_SAFE_NO_PAD.encode(account_digest)
        );
        let ip_key = format!(
            "platform:identity:login:ip:{}",
            URL_SAFE_NO_PAD.encode(ip_digest)
        );
        let mut connection = self.redis.clone();
        let account_count: u64 = redis::cmd("INCR")
            .arg(&account_key)
            .query_async(&mut connection)
            .await
            .map_err(IdentityError::Redis)?;
        let mut connection = self.redis.clone();
        let ip_count: u64 = redis::cmd("INCR")
            .arg(&ip_key)
            .query_async(&mut connection)
            .await
            .map_err(IdentityError::Redis)?;
        if account_count == 1 {
            let mut connection = self.redis.clone();
            let _: bool = connection
                .expire(&account_key, LOGIN_WINDOW_SECONDS as i64)
                .await
                .map_err(IdentityError::Redis)?;
        }
        if ip_count == 1 {
            let mut connection = self.redis.clone();
            let _: bool = connection
                .expire(&ip_key, LOGIN_WINDOW_SECONDS as i64)
                .await
                .map_err(IdentityError::Redis)?;
        }
        if account_count > LOGIN_WINDOW_LIMIT || ip_count > LOGIN_WINDOW_LIMIT * 5 {
            return Err(IdentityError::RateLimited);
        }
        Ok(())
    }
}
