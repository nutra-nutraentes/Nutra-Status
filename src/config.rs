use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Arquivo {
    #[serde(rename = "check", default)]
    pub checks: Vec<CheckDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckDef {
    pub id: String,
    pub nome: String,
    pub grupo: String,
    #[serde(default = "verdadeiro")]
    pub ativo: bool,
    #[serde(flatten)]
    pub tipo: Tipo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "tipo", rename_all = "lowercase")]
pub enum Tipo {
    /// GET em uma URL. OK quando o status HTTP é <= `status_ok_ate` e o JSON
    /// (se `esperar_json` for informado) contém os campos esperados.
    Http {
        url: String,
        #[serde(default)]
        fallback_url: Option<String>,
        #[serde(default = "status_ok_padrao")]
        status_ok_ate: u16,
        #[serde(default)]
        esperar_json: BTreeMap<String, toml::Value>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// Resolve o registro A do host e, se `url` for informada, faz um GET nela.
    Dns {
        host: String,
        #[serde(default)]
        url: Option<String>,
        #[serde(default = "status_ok_dns_padrao")]
        status_ok_ate: u16,
    },
    /// Página de status no formato Atlassian Statuspage (ex.: Cloudflare).
    Statuspage { url: String },
    /// Conexão direta no Postgres usando DB_HOST/DB_PORT/DB_NAME/DB_USERNAME/DB_PASSWORD.
    Postgres,
    /// Autentica com a service account (FIREBASE_SERVICE_ACCOUNT_JSON) e lista 1 usuário.
    Firebase,
}

fn verdadeiro() -> bool {
    true
}

fn status_ok_padrao() -> u16 {
    399
}

fn status_ok_dns_padrao() -> u16 {
    499
}

pub fn carregar(caminho: &Path) -> Result<Vec<CheckDef>> {
    let texto =
        std::fs::read_to_string(caminho).with_context(|| format!("lendo {}", caminho.display()))?;
    parse(&texto)
}

pub fn parse(texto: &str) -> Result<Vec<CheckDef>> {
    let arquivo: Arquivo = toml::from_str(texto).context("checks.toml inválido")?;
    let mut ids = std::collections::HashSet::new();
    for c in &arquivo.checks {
        if !ids.insert(c.id.as_str()) {
            bail!("id de check duplicado: {}", c.id);
        }
    }
    Ok(arquivo.checks.into_iter().filter(|c| c.ativo).collect())
}

/// Substitui `${VAR}` pelo valor da variável de ambiente.
/// Retorna `Err(nome)` com a primeira variável ausente ou vazia.
pub fn expandir(texto: &str) -> std::result::Result<String, String> {
    expandir_com(texto, |nome| std::env::var(nome).ok())
}

pub fn expandir_com(
    texto: &str,
    buscar: impl Fn(&str) -> Option<String>,
) -> std::result::Result<String, String> {
    let mut saida = String::with_capacity(texto.len());
    let mut resto = texto;
    while let Some(inicio) = resto.find("${") {
        saida.push_str(&resto[..inicio]);
        let depois = &resto[inicio + 2..];
        let Some(fim) = depois.find('}') else {
            saida.push_str(&resto[inicio..]);
            return Ok(saida);
        };
        let nome = &depois[..fim];
        match buscar(nome).filter(|v| !v.trim().is_empty()) {
            Some(valor) => saida.push_str(valor.trim().trim_end_matches('/')),
            None => return Err(nome.to_string()),
        }
        resto = &depois[fim + 1..];
    }
    saida.push_str(resto);
    Ok(saida)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn parse_tipos() {
        let checks = parse(
            r#"
            [[check]]
            id = "api"
            nome = "API"
            grupo = "Plataforma"
            tipo = "http"
            url = "https://exemplo/health"
            esperar_json = { status = "ok" }

            [[check]]
            id = "ws"
            nome = "Workspace"
            grupo = "Domínios"
            tipo = "dns"
            host = "workspace.exemplo"
            url = "https://workspace.exemplo"

            [[check]]
            id = "pg"
            nome = "Postgres"
            grupo = "Bancos"
            tipo = "postgres"

            [[check]]
            id = "off"
            nome = "Desligado"
            grupo = "X"
            tipo = "firebase"
            ativo = false
            "#,
        )
        .unwrap();
        assert_eq!(checks.len(), 3);
        match &checks[0].tipo {
            Tipo::Http {
                esperar_json,
                status_ok_ate,
                ..
            } => {
                assert_eq!(*status_ok_ate, 399);
                assert_eq!(esperar_json["status"].as_str(), Some("ok"));
            }
            outro => panic!("tipo inesperado {outro:?}"),
        }
        assert!(matches!(
            checks[1].tipo,
            Tipo::Dns {
                status_ok_ate: 499,
                ..
            }
        ));
        assert!(matches!(checks[2].tipo, Tipo::Postgres));
    }

    #[test]
    fn checks_toml_do_repositorio_e_valido() {
        let checks = parse(include_str!("../config/checks.toml")).unwrap();
        assert!(checks.iter().any(|c| c.id == "api"));
    }

    #[test]
    fn rejeita_id_duplicado() {
        let erro = parse(
            r#"
            [[check]]
            id = "a"
            nome = "A"
            grupo = "G"
            tipo = "postgres"
            [[check]]
            id = "a"
            nome = "B"
            grupo = "G"
            tipo = "firebase"
            "#,
        );
        assert!(erro.is_err());
    }

    #[test]
    fn expande_variaveis() {
        let env = |n: &str| match n {
            "URL" => Some("http://host:8001/".to_string()),
            "VAZIA" => Some("  ".to_string()),
            _ => None,
        };
        assert_eq!(
            expandir_com("${URL}/health", env).unwrap(),
            "http://host:8001/health"
        );
        assert_eq!(expandir_com("sem variavel", env).unwrap(), "sem variavel");
        assert_eq!(expandir_com("${FALTA}/x", env).unwrap_err(), "FALTA");
        assert_eq!(expandir_com("${VAZIA}", env).unwrap_err(), "VAZIA");
    }
}
