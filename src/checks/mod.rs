mod dns;
mod firebase;
mod http;
mod postgres;
mod statuspage;

use std::time::Duration;

use anyhow::Result;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;

use crate::config::{expandir, Tipo};
use crate::model::Status;

/// O que uma checagem devolve (a latência é medida pelo runner).
#[derive(Debug, Clone)]
pub struct Saida {
    pub status: Status,
    pub detalhe: Option<String>,
}

impl Saida {
    pub fn online() -> Self {
        Self {
            status: Status::Online,
            detalhe: None,
        }
    }

    pub fn com(status: Status, detalhe: impl Into<String>) -> Self {
        Self {
            status,
            detalhe: Some(detalhe.into()),
        }
    }

    pub fn offline(detalhe: impl Into<String>) -> Self {
        Self::com(Status::Offline, detalhe)
    }

    pub fn sem_secret(nome: &str) -> Self {
        Self::com(Status::Ignorado, format!("secret {nome} não configurado"))
    }
}

/// Recursos compartilhados entre as checagens.
pub struct Contexto {
    pub http: reqwest::Client,
    pub dns: TokioAsyncResolver,
}

impl Contexto {
    pub fn novo(timeout: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .user_agent(concat!("nutra-status/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let mut opcoes = ResolverOpts::default();
        opcoes.timeout = Duration::from_secs(5);
        opcoes.attempts = 2;
        // Resolver público independente (Google), para não depender do DNS da Cloudflare.
        let dns = TokioAsyncResolver::tokio(ResolverConfig::google(), opcoes);
        Ok(Self { http, dns })
    }
}

pub async fn executar(tipo: &Tipo, ctx: &Contexto) -> Saida {
    match tipo {
        Tipo::Http {
            url,
            fallback_url,
            status_ok_ate,
            esperar_json,
            headers,
        } => {
            let url = match expandir(url) {
                Ok(u) => u,
                Err(var) => return Saida::sem_secret(&var),
            };
            let mut cabecalhos = Vec::with_capacity(headers.len());
            for (nome, valor) in headers {
                match expandir(valor) {
                    Ok(v) => cabecalhos.push((nome.clone(), v)),
                    Err(var) => return Saida::sem_secret(&var),
                }
            }
            let saida = http::checar(ctx, &url, *status_ok_ate, esperar_json, &cabecalhos).await;
            match fallback_url {
                Some(alternativa) if saida.status != Status::Online => {
                    let Ok(alternativa) = expandir(alternativa) else {
                        return saida;
                    };
                    let segunda =
                        http::checar(ctx, &alternativa, *status_ok_ate, esperar_json, &cabecalhos)
                            .await;
                    if segunda.status == Status::Online {
                        segunda
                    } else {
                        saida
                    }
                }
                _ => saida,
            }
        }
        Tipo::Dns {
            host,
            url,
            status_ok_ate,
        } => dns::checar(ctx, host, url.as_deref(), *status_ok_ate).await,
        Tipo::Statuspage { url } => statuspage::checar(ctx, url).await,
        Tipo::Postgres => postgres::checar().await,
        Tipo::Firebase => firebase::checar(ctx).await,
    }
}

/// Descreve um erro do reqwest com a cadeia de causas (ex.: "falha de conexão: dns error: ...").
pub fn descrever_erro_http(erro: &reqwest::Error) -> String {
    let mut msg = if erro.is_timeout() {
        "timeout".to_string()
    } else if erro.is_connect() {
        "falha de conexão".to_string()
    } else {
        "erro na requisição".to_string()
    };
    let mut fonte = std::error::Error::source(erro);
    while let Some(f) = fonte {
        msg.push_str(": ");
        msg.push_str(&f.to_string());
        fonte = f.source();
    }
    msg
}
