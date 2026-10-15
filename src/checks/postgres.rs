use std::time::Duration;

use anyhow::{Context, Result};
use postgres_native_tls::MakeTlsConnector;
use tokio_postgres::config::SslMode;

use super::Saida;

/// Mesmas variáveis da Nutra-API2 (app/config.py e app/database.py):
/// DB_HOST, DB_PORT, DB_NAME, DB_USERNAME, DB_PASSWORD, DB_SSL (padrão true) e
/// DB_SSL_CA (PEM em base64, opcional).
pub async fn checar() -> Saida {
    let var = |nome: &str| std::env::var(nome).ok().filter(|v| !v.trim().is_empty());
    let mut faltando = None;
    let mut obrigatoria = |nome: &'static str| {
        let valor = var(nome);
        if valor.is_none() && faltando.is_none() {
            faltando = Some(nome);
        }
        valor.unwrap_or_default()
    };
    let host = obrigatoria("DB_HOST");
    let banco = obrigatoria("DB_NAME");
    let usuario = obrigatoria("DB_USERNAME");
    let senha = obrigatoria("DB_PASSWORD");
    if let Some(nome) = faltando {
        return Saida::sem_secret(nome);
    }
    let porta = var("DB_PORT")
        .and_then(|p| p.parse().ok())
        .unwrap_or(5432u16);
    let ssl = var("DB_SSL")
        .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no" | "disable"))
        .unwrap_or(true);

    match conectar(
        &host,
        porta,
        &banco,
        &usuario,
        &senha,
        ssl,
        var("DB_SSL_CA"),
    )
    .await
    {
        Ok(()) => Saida::online(),
        Err(e) => Saida::offline(format!("{e:#}")),
    }
}

async fn conectar(
    host: &str,
    porta: u16,
    banco: &str,
    usuario: &str,
    senha: &str,
    ssl: bool,
    ca_base64: Option<String>,
) -> Result<()> {
    let mut cfg = tokio_postgres::Config::new();
    cfg.host(host)
        .port(porta)
        .dbname(banco)
        .user(usuario)
        .password(senha)
        .application_name("nutra-status")
        .connect_timeout(Duration::from_secs(8))
        .ssl_mode(if ssl {
            SslMode::Require
        } else {
            SslMode::Disable
        });

    let mut tls = native_tls::TlsConnector::builder();
    // Igual à API: com CA valida a cadeia mas não o hostname; sem CA aceita qualquer certificado.
    tls.danger_accept_invalid_hostnames(true);
    match ca_base64 {
        Some(ca) => {
            use base64_decode::decodificar;
            let pem = decodificar(ca.trim())
                .map_err(|_| anyhow::anyhow!("DB_SSL_CA não é base64 válido"))?;
            let cert = native_tls::Certificate::from_pem(&pem).context("DB_SSL_CA inválido")?;
            tls.add_root_certificate(cert);
        }
        None => {
            tls.danger_accept_invalid_certs(true);
        }
    }
    let conector = MakeTlsConnector::new(tls.build()?);

    let (cliente, conexao) = cfg.connect(conector).await.context("conexão")?;
    let tarefa = tokio::spawn(conexao);
    cliente.simple_query("SELECT 1").await.context("SELECT 1")?;
    drop(cliente);
    let _ = tarefa.await;
    Ok(())
}

/// Decodificador base64 mínimo (evita uma dependência só para isto).
mod base64_decode {
    pub fn decodificar(texto: &str) -> Result<Vec<u8>, ()> {
        let valor = |c: u8| -> Result<u32, ()> {
            Ok(match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' | b'-' => 62,
                b'/' | b'_' => 63,
                _ => return Err(()),
            } as u32)
        };
        let limpos: Vec<u8> = texto
            .bytes()
            .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
            .collect();
        let mut saida = Vec::with_capacity(limpos.len() * 3 / 4);
        for bloco in limpos.chunks(4) {
            let mut acc = 0u32;
            for (i, &c) in bloco.iter().enumerate() {
                acc |= valor(c)? << (18 - 6 * i);
            }
            let bytes = acc.to_be_bytes();
            saida.extend_from_slice(&bytes[1..bloco.len()]);
        }
        Ok(saida)
    }

    #[cfg(test)]
    mod testes {
        #[test]
        fn decodifica() {
            assert_eq!(super::decodificar("aGVsbG8=").unwrap(), b"hello");
            assert_eq!(super::decodificar("aGk=").unwrap(), b"hi");
            assert_eq!(super::decodificar("YWJj").unwrap(), b"abc");
            assert!(super::decodificar("@@").is_err());
        }
    }
}
