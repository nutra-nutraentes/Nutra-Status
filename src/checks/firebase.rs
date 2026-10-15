use anyhow::{bail, Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use super::{Contexto, Saida};

#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    #[serde(default = "token_uri_padrao")]
    token_uri: String,
}

fn token_uri_padrao() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
}

/// Equivalente ao `auth.list_users(max_results=1)` da Nutra-API2: autentica com a
/// service account e lista um usuário no Firebase Auth (Identity Toolkit).
pub async fn checar(ctx: &Contexto) -> Saida {
    let Some(json) = std::env::var("FIREBASE_SERVICE_ACCOUNT_JSON")
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        return Saida::sem_secret("FIREBASE_SERVICE_ACCOUNT_JSON");
    };
    match listar_um_usuario(ctx, &json).await {
        Ok(()) => Saida::online(),
        Err(e) => Saida::offline(format!("{e:#}")),
    }
}

async fn listar_um_usuario(ctx: &Contexto, json: &str) -> Result<()> {
    let sa: ServiceAccount =
        serde_json::from_str(json).context("FIREBASE_SERVICE_ACCOUNT_JSON inválido")?;
    let agora = chrono::Utc::now().timestamp();
    let claims = Claims {
        iss: &sa.client_email,
        scope: "https://www.googleapis.com/auth/cloud-platform",
        aud: &sa.token_uri,
        iat: agora,
        exp: agora + 600,
    };
    let chave = EncodingKey::from_rsa_pem(sa.private_key.as_bytes()).context("private_key")?;
    let assinatura = encode(&Header::new(Algorithm::RS256), &claims, &chave)?;

    let resposta = ctx
        .http
        .post(&sa.token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assinatura.as_str()),
        ])
        .send()
        .await
        .context("token OAuth")?;
    if !resposta.status().is_success() {
        bail!("token OAuth: HTTP {}", resposta.status().as_u16());
    }
    let token: Token = resposta.json().await.context("token OAuth")?;

    let url = format!(
        "https://identitytoolkit.googleapis.com/v1/projects/{}/accounts:batchGet?maxResults=1",
        sa.project_id
    );
    let resposta = ctx
        .http
        .get(url)
        .bearer_auth(token.access_token)
        .send()
        .await
        .context("Identity Toolkit")?;
    if !resposta.status().is_success() {
        bail!("Identity Toolkit: HTTP {}", resposta.status().as_u16());
    }
    Ok(())
}
