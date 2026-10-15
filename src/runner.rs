use std::time::{Duration, Instant};

use futures::future::join_all;

use crate::checks::{self, Contexto, Saida};
use crate::config::CheckDef;
use crate::model::{Resultado, Status};

pub struct Opcoes {
    pub timeout: Duration,
    pub espera_retentativa: Duration,
}

/// Executa todas as checagens em paralelo. Uma checagem com problema é repetida
/// uma vez após `espera_retentativa`, para não alarmar por oscilação momentânea.
pub async fn executar_todos(defs: &[CheckDef], ctx: &Contexto, op: &Opcoes) -> Vec<Resultado> {
    join_all(defs.iter().map(|def| executar_um(def, ctx, op))).await
}

async fn executar_um(def: &CheckDef, ctx: &Contexto, op: &Opcoes) -> Resultado {
    let (mut saida, mut ms) = medir(def, ctx, op.timeout).await;
    if saida.status.com_problema() {
        tokio::time::sleep(op.espera_retentativa).await;
        (saida, ms) = medir(def, ctx, op.timeout).await;
    }
    Resultado {
        id: def.id.clone(),
        nome: def.nome.clone(),
        grupo: def.grupo.clone(),
        status: saida.status,
        latencia_ms: (saida.status != Status::Ignorado).then_some(ms),
        detalhe: saida.detalhe,
    }
}

async fn medir(def: &CheckDef, ctx: &Contexto, timeout: Duration) -> (Saida, u64) {
    let inicio = Instant::now();
    let saida = match tokio::time::timeout(timeout, checks::executar(&def.tipo, ctx)).await {
        Ok(s) => s,
        Err(_) => Saida::offline(format!("timeout de {}s", timeout.as_secs())),
    };
    (saida, inicio.elapsed().as_millis() as u64)
}
