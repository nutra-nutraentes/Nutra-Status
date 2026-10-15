use serde::Deserialize;

use super::{descrever_erro_http, Contexto, Saida};
use crate::model::Status;

#[derive(Deserialize)]
struct Resposta {
    status: Indicador,
}

#[derive(Deserialize)]
struct Indicador {
    indicator: String,
    #[serde(default)]
    description: String,
}

pub async fn checar(ctx: &Contexto, url: &str) -> Saida {
    let resposta = match ctx.http.get(url).send().await {
        Ok(r) => r,
        Err(e) => return Saida::offline(descrever_erro_http(&e)),
    };
    if !resposta.status().is_success() {
        return Saida::offline(format!("HTTP {}", resposta.status().as_u16()));
    }
    match resposta.json::<Resposta>().await {
        Ok(r) => avaliar(&r.status.indicator, &r.status.description),
        Err(e) => Saida::offline(format!("resposta inesperada: {e}")),
    }
}

/// Mapeia o indicador do Statuspage: none → online, minor/major/maintenance → degradado
/// (problemas regionais são frequentes), critical → offline.
pub fn avaliar(indicador: &str, descricao: &str) -> Saida {
    let status = match indicador {
        "none" => return Saida::online(),
        "minor" | "major" | "maintenance" => Status::Degradado,
        _ => Status::Offline,
    };
    Saida::com(status, format!("{indicador}: {descricao}"))
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn indicadores() {
        assert_eq!(avaliar("none", "").status, Status::Online);
        assert_eq!(avaliar("minor", "x").status, Status::Degradado);
        assert_eq!(avaliar("major", "x").status, Status::Degradado);
        assert_eq!(avaliar("critical", "x").status, Status::Offline);
    }
}
