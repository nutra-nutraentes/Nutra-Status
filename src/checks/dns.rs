use super::{http, Contexto, Saida};
use crate::model::Status;

pub async fn checar(ctx: &Contexto, host: &str, url: Option<&str>, status_ok_ate: u16) -> Saida {
    let ips: Vec<String> = match ctx.dns.ipv4_lookup(host).await {
        Ok(resposta) => resposta.iter().map(|a| a.to_string()).collect(),
        Err(e) => return Saida::offline(format!("registro A não resolve: {e}")),
    };
    if ips.is_empty() {
        return Saida::offline("registro A vazio");
    }
    let lista = ips.join(", ");
    let Some(url) = url else {
        return Saida {
            status: Status::Online,
            detalhe: Some(format!("A: {lista}")),
        };
    };
    let saida = http::checar(ctx, url, status_ok_ate, &Default::default(), &[]).await;
    let detalhe = match saida.detalhe {
        Some(d) => format!("A: {lista}; {d}"),
        None => format!("A: {lista}"),
    };
    Saida {
        status: saida.status,
        detalhe: Some(detalhe),
    }
}
