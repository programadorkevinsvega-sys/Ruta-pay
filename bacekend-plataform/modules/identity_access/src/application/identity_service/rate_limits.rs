use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
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
        const RATE_LIMIT_SCRIPT: &str = r#"
            local account_count = redis.call('INCR', KEYS[1])
            if account_count == 1 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            local ip_count = redis.call('INCR', KEYS[2])
            if ip_count == 1 then
                redis.call('EXPIRE', KEYS[2], ARGV[1])
            end
            return {account_count, ip_count}
        "#;
        let mut connection = self.redis.clone();
        let (account_count, ip_count): (i64, i64) = redis::cmd("EVAL")
            .arg(RATE_LIMIT_SCRIPT)
            .arg(2)
            .arg(&account_key)
            .arg(&ip_key)
            .arg(LOGIN_WINDOW_SECONDS)
            .query_async(&mut connection)
            .await
            .map_err(IdentityError::Redis)?;
        if account_count > LOGIN_WINDOW_LIMIT as i64 || ip_count > (LOGIN_WINDOW_LIMIT * 5) as i64 {
            return Err(IdentityError::RateLimited);
        }
        Ok(())
    }
}
