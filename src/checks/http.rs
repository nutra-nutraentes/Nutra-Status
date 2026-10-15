use std::collections::BTreeMap;

use super::{descrever_erro_http, Contexto, Saida};

pub async fn checar(
    ctx: &Contexto,
    url: &str,
    status_ok_ate: u16,
    esperar_json: &BTreeMap<String, toml::Value>,
    headers: &[(String, String)],
) -> Saida {
    let mut req = ctx.http.get(url);
    for (nome, valor) in headers {
        req = req.header(nome, valor);
    }
    let resposta = match req.send().await {
        Ok(r) => r,
        Err(e) => return Saida::offline(descrever_erro_http(&e)),
    };
    let codigo = resposta.status().as_u16();
    // O corpo só é necessário quando há campos a validar.
    let corpo = if esperar_json.is_empty() {
        String::new()
    } else {
        resposta.text().await.unwrap_or_default()
    };
    avaliar_resposta(codigo, &corpo, status_ok_ate, esperar_json)
}

/// Regra pura de avaliação, separada para teste.
pub fn avaliar_resposta(
    codigo: u16,
    corpo: &str,
    status_ok_ate: u16,
    esperar_json: &BTreeMap<String, toml::Value>,
) -> Saida {
    if codigo > status_ok_ate {
        return Saida::offline(format!("HTTP {codigo}"));
    }
    if esperar_json.is_empty() {
        return Saida::online();
    }
    let json: serde_json::Value = match serde_json::from_str(corpo) {
        Ok(v) => v,
        Err(_) => return Saida::offline(format!("HTTP {codigo}, resposta não é JSON")),
    };
    for (campo, esperado) in esperar_json {
        let esperado = serde_json::to_value(esperado).unwrap_or(serde_json::Value::Null);
        let recebido = json.get(campo).cloned().unwrap_or(serde_json::Value::Null);
        if recebido != esperado {
            return Saida::offline(format!(
                "HTTP {codigo}, campo `{campo}` = {recebido} (esperado {esperado})"
            ));
        }
    }
    Saida::online()
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::model::Status;

    fn esperado(pares: &[(&str, toml::Value)]) -> BTreeMap<String, toml::Value> {
        pares
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn status_http() {
        let vazio = BTreeMap::new();
        assert_eq!(
            avaliar_resposta(200, "", 399, &vazio).status,
            Status::Online
        );
        assert_eq!(
            avaliar_resposta(301, "", 399, &vazio).status,
            Status::Online
        );
        assert_eq!(
            avaliar_resposta(404, "", 399, &vazio).status,
            Status::Offline
        );
        assert_eq!(
            avaliar_resposta(404, "", 499, &vazio).status,
            Status::Online
        );
        assert_eq!(
            avaliar_resposta(502, "", 499, &vazio).status,
            Status::Offline
        );
    }

    #[test]
    fn campos_json() {
        let e = esperado(&[("banco", toml::Value::Boolean(true))]);
        assert_eq!(
            avaliar_resposta(200, r#"{"status":"ok","banco":true}"#, 399, &e).status,
            Status::Online
        );
        assert_eq!(
            avaliar_resposta(200, r#"{"status":"ok","banco":false}"#, 399, &e).status,
            Status::Offline
        );
        assert_eq!(
            avaliar_resposta(200, "<html>", 399, &e).status,
            Status::Offline
        );
        let e = esperado(&[("status", toml::Value::String("ok".into()))]);
        assert_eq!(
            avaliar_resposta(200, r#"{"status":"ok"}"#, 399, &e).status,
            Status::Online
        );
    }
}
