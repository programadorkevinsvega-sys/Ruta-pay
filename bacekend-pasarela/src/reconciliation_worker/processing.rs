use super::*;

pub(super) async fn process_request(
    state: &GatewayWebhookState,
    jetstream: &jetstream::Context,
    config: &ReconciliationWorkerConfig,
    raw: &[u8],
    delivery_attempt: i64,
) -> Result<(), RequestError> {
    let value: Value = serde_json::from_slice(raw).map_err(|_| RequestError::InvalidEnvelope)?;
    verify_platform_event(&value, config)?;
    let envelope: PlatformEvent =
        serde_json::from_value(value).map_err(|_| RequestError::InvalidEnvelope)?;
    if envelope.event_type != "payment.reconciliation.requested"
        || envelope.audience != "gateway"
        || envelope.correlation_id.is_empty()
        || envelope.correlation_id.len() > 128
    {
        return Err(RequestError::InvalidEnvelope);
    }
    let request: ReconciliationRequest =
        serde_json::from_value(envelope.payload).map_err(|_| RequestError::InvalidEnvelope)?;
    if request.tenant_id.is_nil()
        || request.payment_intent_id.is_nil()
        || request.attempt_id.is_nil()
        || request.reconciliation_case_id.is_nil()
        || request.environment != "test" && request.environment != "live"
    {
        return Err(RequestError::InvalidEnvelope);
    }
    let provider_code = request.provider_code.as_deref().unwrap_or("");
    let account_reference = request
        .provider_account_reference
        .as_deref()
        .ok_or(RequestError::InvalidEnvelope)?;
    let account = state
        .accounts
        .get(account_reference)
        .filter(|account| {
            account.tenant_id == request.tenant_id
                && account.environment == request.environment
                && provider_code == "wompi"
        })
        .ok_or(RequestError::InvalidAccount)?;

    let Some(transaction_reference) = request.provider_transaction_reference.as_deref() else {
        return publish_result(
            state,
            jetstream,
            &request,
            account,
            envelope.correlation_id.as_str(),
            None,
            "manual_review",
            None,
            None,
            None,
            Some("provider_transaction_reference_missing"),
        )
        .await;
    };
    let client = WompiClient::new(
        WompiEnvironment::parse(&request.environment).map_err(|_| RequestError::InvalidAccount)?,
        account.private_key.clone(),
    )
    .map_err(|_| RequestError::InvalidAccount)?;
    let transaction = match client.lookup_transaction(transaction_reference).await {
        Ok(transaction) => transaction,
        Err(WompiClientError::Unavailable) if delivery_attempt < 10 => {
            return Err(RequestError::Unavailable);
        }
        Err(WompiClientError::Unavailable) => {
            return publish_result(
                state,
                jetstream,
                &request,
                account,
                envelope.correlation_id.as_str(),
                Some(transaction_reference),
                "manual_review",
                None,
                None,
                None,
                Some("provider_lookup_attempts_exhausted"),
            )
            .await;
        }
        Err(WompiClientError::Rejected | WompiClientError::InvalidResponse) => {
            return publish_result(
                state,
                jetstream,
                &request,
                account,
                envelope.correlation_id.as_str(),
                Some(transaction_reference),
                "manual_review",
                None,
                None,
                None,
                Some("provider_lookup_untrusted"),
            )
            .await;
        }
        Err(WompiClientError::InvalidCredential | WompiClientError::InvalidTransactionId) => {
            return Err(RequestError::InvalidAccount);
        }
    };
    let expected_reference = reference_for_attempt(request.attempt_id);
    if transaction.reference != expected_reference
        || request.amount_minor != Some(transaction.amount_minor)
        || request.currency.as_deref() != Some(transaction.currency.as_str())
    {
        return publish_result(
            state,
            jetstream,
            &request,
            account,
            envelope.correlation_id.as_str(),
            Some(transaction_reference),
            "manual_review",
            None,
            None,
            None,
            Some("provider_evidence_mismatch"),
        )
        .await;
    }
    if transaction.status == WompiPaymentStatus::Processing {
        if delivery_attempt < 10 {
            return Err(RequestError::Unavailable);
        }
        return publish_result(
            state,
            jetstream,
            &request,
            account,
            envelope.correlation_id.as_str(),
            Some(transaction_reference),
            "manual_review",
            None,
            Some(transaction.amount_minor),
            None,
            Some("provider_transaction_still_processing"),
        )
        .await;
    }
    let status = transaction.status.as_platform_status();
    let evidence = serde_json::to_vec(&serde_json::json!({
        "id": transaction.id,
        "reference": transaction.reference,
        "status": transaction.status.as_platform_status(),
        "amount_minor": transaction.amount_minor,
        "currency": transaction.currency
    }))
    .map_err(|_| RequestError::InvalidEnvelope)?;
    let evidence_hash = hex_upper(&Sha256::digest(evidence));
    publish_result(
        state,
        jetstream,
        &request,
        account,
        envelope.correlation_id.as_str(),
        Some(transaction_reference),
        "resolved",
        Some(status),
        Some(transaction.amount_minor),
        Some((&transaction.currency, evidence_hash)),
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn publish_result(
    state: &GatewayWebhookState,
    jetstream: &jetstream::Context,
    request: &ReconciliationRequest,
    account: &WompiAccountConfig,
    correlation_id: &str,
    transaction_reference: Option<&str>,
    outcome: &'static str,
    status: Option<&'static str>,
    amount_minor: Option<i64>,
    evidence: Option<(&str, String)>,
    safe_error_code: Option<&'static str>,
) -> Result<(), RequestError> {
    let event = ReconciliationResult {
        schema_version: 1,
        event_id: reconciliation_event_id(request.reconciliation_case_id, outcome, status),
        event_type: "payment.provider_reconciliation.result",
        provider_code: "wompi",
        provider_account_reference: &account.account_reference,
        tenant_id: request.tenant_id,
        environment: &request.environment,
        payment_intent_id: request.payment_intent_id,
        payment_attempt_id: request.attempt_id,
        reconciliation_case_id: request.reconciliation_case_id,
        provider_transaction_reference: transaction_reference,
        outcome,
        status,
        amount_minor,
        currency: evidence.as_ref().map(|value| value.0),
        evidence_sha256: evidence.map(|value| value.1),
        safe_error_code,
        correlation_id,
        occurred_at_unix_millis: chrono::Utc::now().timestamp_millis(),
    };
    let unsigned = serde_json::to_vec(&event).map_err(|_| RequestError::InvalidEnvelope)?;
    let signature = hex_upper(hmac::sign(&state.event_signing_key, &unsigned).as_ref());
    let signed = SignedResult {
        key_id: &state.signing_key_id,
        event: &event,
        signature,
    };
    let payload = serde_json::to_vec(&signed).map_err(|_| RequestError::InvalidEnvelope)?;
    let mut headers = async_nats::HeaderMap::new();
    headers.append("Nats-Msg-Id", event.event_id.to_string());
    headers.append("Payvia-Event-Type", event.event_type);
    headers.append("Payvia-Producer", "gateway-pasarela");
    let published = jetstream
        .publish_with_headers(PLATFORM_RESULT_SUBJECT, headers, payload.into())
        .await
        .map_err(|_| RequestError::Unavailable)?;
    published.await.map_err(|_| RequestError::Unavailable)?;
    tracing::info!(
        event.name = "gateway.reconciliation.result.published",
        component = "gateway_reconciliation",
        operation = "publish_authoritative_reconciliation_result",
        provider = "wompi",
        reconciliation_case_id = %request.reconciliation_case_id,
        payment_attempt_id = %request.attempt_id,
        outcome,
        result = "durably_published",
        correlation_id
    );
    Ok(())
}

fn verify_platform_event(
    value: &Value,
    config: &ReconciliationWorkerConfig,
) -> Result<(), RequestError> {
    let mut unsigned = value.clone();
    let auth = unsigned
        .as_object_mut()
        .and_then(|object| object.remove("producer_auth"))
        .ok_or(RequestError::Unauthenticated)?;
    let auth: ProducerAuth =
        serde_json::from_value(auth).map_err(|_| RequestError::Unauthenticated)?;
    if auth.key_id != config.platform_request_key_id.as_ref() {
        return Err(RequestError::Unauthenticated);
    }
    let signature = decode_hex(&auth.signature).ok_or(RequestError::Unauthenticated)?;
    let payload = serde_json::to_vec(&unsigned).map_err(|_| RequestError::InvalidEnvelope)?;
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, &config.platform_request_key),
        &payload,
        &signature,
    )
    .map_err(|_| RequestError::Unauthenticated)
}

fn reconciliation_event_id(case_id: Uuid, outcome: &str, status: Option<&str>) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(case_id.as_bytes());
    digest.update(outcome.as_bytes());
    digest.update(status.unwrap_or("none").as_bytes());
    let bytes = digest.finalize();
    let mut id = [0_u8; 16];
    id.copy_from_slice(&bytes[..16]);
    id[6] = (id[6] & 0x0f) | 0x50;
    id[8] = (id[8] & 0x3f) | 0x80;
    Uuid::from_bytes(id)
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn hex_upper(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
