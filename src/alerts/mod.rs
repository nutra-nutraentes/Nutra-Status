mod email;
mod github_issue;

use chrono::{DateTime, Duration, Utc};

use crate::model::{EstadoServico, Incidente, Resultado, Status};
use crate::store::Dados;

pub use email::Email;
pub use github_issue::GitHub;

/// Intervalo entre lembretes enquanto um serviço continua fora.
pub const INTERVALO_LEMBRETE_HORAS: i64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evento {
    /// Estava online (ou nunca foi visto) e agora tem problema.
    Caiu,
    /// Continua com problema mas mudou entre degradado e offline.
    Mudou { piorou: bool },
    /// Continua com problema e já passou o intervalo de lembrete.
    Lembrete,
    /// Estava com problema e voltou.
    Voltou { duracao_s: i64 },
}

/// Decide o evento de alerta comparando o estado anterior com o status atual.
pub fn decidir(
    anterior: Option<&EstadoServico>,
    atual: Status,
    agora: DateTime<Utc>,
) -> Option<Evento> {
    if atual == Status::Ignorado {
        return None;
    }
    let anterior_com_problema = anterior.filter(|e| e.status.com_problema());
    match (anterior_com_problema, atual.com_problema()) {
        (None, true) => Some(Evento::Caiu),
        (None, false) => None,
        (Some(e), false) => Some(Evento::Voltou {
            duracao_s: (agora - e.desde).num_seconds().max(0),
        }),
        (Some(e), true) if e.status != atual => Some(Evento::Mudou {
            piorou: atual.gravidade() > e.status.gravidade(),
        }),
        (Some(e), true) => {
            let ultimo = e.ultimo_aviso.unwrap_or(e.desde);
            (agora - ultimo >= Duration::hours(INTERVALO_LEMBRETE_HORAS))
                .then_some(Evento::Lembrete)
        }
    }
}

/// Um alerta a ser enviado. Carrega o `detalhe`, que só pode ir para o e-mail.
#[derive(Debug, Clone)]
pub struct Aviso {
    pub id: String,
    pub nome: String,
    pub status: Status,
    pub evento: Evento,
    pub detalhe: Option<String>,
    pub desde: DateTime<Utc>,
    pub issue: Option<u64>,
}

/// Atualiza estado e incidentes conforme os resultados e devolve os avisos a enviar.
pub fn aplicar(dados: &mut Dados, resultados: &[Resultado], agora: DateTime<Utc>) -> Vec<Aviso> {
    let mut avisos = Vec::new();
    for r in resultados {
        let anterior = dados.estado.get(&r.id).cloned();
        let evento = decidir(anterior.as_ref(), r.status, agora);

        match evento {
            None => {
                if r.status != Status::Ignorado && anterior.is_none() {
                    dados
                        .estado
                        .insert(r.id.clone(), novo_estado(r.status, agora));
                }
                continue;
            }
            Some(Evento::Caiu) => {
                let incidente_id = format!("{}-{}", r.id, agora.timestamp());
                dados.incidentes.push(Incidente {
                    id: incidente_id.clone(),
                    servico: r.id.clone(),
                    nome: r.nome.clone(),
                    status: r.status,
                    inicio: agora,
                    fim: None,
                    duracao_s: None,
                    issue: None,
                });
                dados.estado.insert(
                    r.id.clone(),
                    EstadoServico {
                        status: r.status,
                        desde: agora,
                        ultimo_aviso: Some(agora),
                        issue: None,
                        incidente: Some(incidente_id),
                    },
                );
            }
            Some(Evento::Mudou { .. }) | Some(Evento::Lembrete) => {
                let estado = dados.estado.get_mut(&r.id).expect("estado anterior existe");
                estado.status = r.status;
                estado.ultimo_aviso = Some(agora);
                if let Some(inc) = incidente_aberto(&mut dados.incidentes, estado) {
                    if r.status.gravidade() > inc.status.gravidade() {
                        inc.status = r.status;
                    }
                }
            }
            Some(Evento::Voltou { duracao_s }) => {
                if let Some(estado) = dados.estado.get(&r.id).cloned() {
                    if let Some(inc) = incidente_aberto(&mut dados.incidentes, &estado) {
                        inc.fim = Some(agora);
                        inc.duracao_s = Some(duracao_s);
                    }
                }
                let issue = anterior.as_ref().and_then(|e| e.issue);
                dados
                    .estado
                    .insert(r.id.clone(), novo_estado(r.status, agora));
                avisos.push(Aviso {
                    id: r.id.clone(),
                    nome: r.nome.clone(),
                    status: r.status,
                    evento: Evento::Voltou { duracao_s },
                    detalhe: r.detalhe.clone(),
                    desde: anterior.map(|e| e.desde).unwrap_or(agora),
                    issue,
                });
                continue;
            }
        }

        let estado = &dados.estado[&r.id];
        avisos.push(Aviso {
            id: r.id.clone(),
            nome: r.nome.clone(),
            status: r.status,
            evento: evento.expect("evento presente"),
            detalhe: r.detalhe.clone(),
            desde: estado.desde,
            issue: estado.issue,
        });
    }
    avisos
}

fn novo_estado(status: Status, agora: DateTime<Utc>) -> EstadoServico {
    EstadoServico {
        status,
        desde: agora,
        ultimo_aviso: None,
        issue: None,
        incidente: None,
    }
}

fn incidente_aberto<'a>(
    incidentes: &'a mut [Incidente],
    estado: &EstadoServico,
) -> Option<&'a mut Incidente> {
    let id = estado.incidente.as_deref()?;
    incidentes.iter_mut().rev().find(|i| i.id == id)
}

/// Envia os avisos: uma issue por serviço no GitHub e um único e-mail por rodada.
/// Falhas de envio são registradas no log e não interrompem a rodada.
pub async fn notificar(
    avisos: &[Aviso],
    dados: &mut Dados,
    github: Option<&GitHub>,
    email: Option<&Email>,
) {
    if avisos.is_empty() {
        return;
    }

    if let Some(gh) = github {
        for aviso in avisos {
            match gh.processar(aviso).await {
                Ok(Some(numero)) => registrar_issue(dados, &aviso.id, numero),
                Ok(None) => {}
                Err(e) => eprintln!(
                    "⚠️  GitHub: falha ao atualizar issue de {}: {e:#}",
                    aviso.id
                ),
            }
        }
    } else {
        eprintln!("ℹ️  GitHub: GITHUB_TOKEN/GITHUB_REPOSITORY ausentes, issues não serão abertas");
    }

    let para_email: Vec<&Aviso> = avisos
        .iter()
        .filter(|a| {
            matches!(
                a.evento,
                Evento::Caiu | Evento::Voltou { .. } | Evento::Mudou { piorou: true }
            )
        })
        .collect();
    if para_email.is_empty() {
        return;
    }
    match email {
        Some(e) => {
            if let Err(erro) = e.enviar(&para_email).await {
                eprintln!("⚠️  E-mail: falha ao enviar alerta: {erro:#}");
            }
        }
        None => eprintln!("ℹ️  E-mail: EMAIL_* ausentes, alerta não enviado"),
    }
}

fn registrar_issue(dados: &mut Dados, id: &str, numero: u64) {
    if let Some(estado) = dados.estado.get_mut(id) {
        if estado.status.com_problema() {
            estado.issue = Some(numero);
            let estado = estado.clone();
            if let Some(inc) = incidente_aberto(&mut dados.incidentes, &estado) {
                inc.issue = Some(numero);
            }
        }
    }
}

pub fn formatar_duracao(segundos: i64) -> String {
    let min = segundos / 60;
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 15, h, 0, 0).unwrap()
    }

    fn estado(status: Status, desde: u32, aviso: Option<u32>) -> EstadoServico {
        EstadoServico {
            status,
            desde: t(desde),
            ultimo_aviso: aviso.map(t),
            issue: None,
            incidente: None,
        }
    }

    #[test]
    fn transicoes() {
        assert_eq!(decidir(None, Status::Online, t(1)), None);
        assert_eq!(decidir(None, Status::Offline, t(1)), Some(Evento::Caiu));
        let on = estado(Status::Online, 0, None);
        assert_eq!(decidir(Some(&on), Status::Online, t(1)), None);
        assert_eq!(
            decidir(Some(&on), Status::Degradado, t(1)),
            Some(Evento::Caiu)
        );
        assert_eq!(decidir(Some(&on), Status::Ignorado, t(1)), None);

        let off = estado(Status::Offline, 1, Some(1));
        assert_eq!(
            decidir(Some(&off), Status::Online, t(3)),
            Some(Evento::Voltou { duracao_s: 7200 })
        );
        assert_eq!(decidir(Some(&off), Status::Offline, t(3)), None);
        assert_eq!(
            decidir(Some(&off), Status::Offline, t(7)),
            Some(Evento::Lembrete)
        );
        assert_eq!(decidir(Some(&off), Status::Ignorado, t(7)), None);
        assert_eq!(
            decidir(Some(&off), Status::Degradado, t(2)),
            Some(Evento::Mudou { piorou: false })
        );
        let deg = estado(Status::Degradado, 1, Some(1));
        assert_eq!(
            decidir(Some(&deg), Status::Offline, t(2)),
            Some(Evento::Mudou { piorou: true })
        );
    }

    #[test]
    fn duracao() {
        assert_eq!(formatar_duracao(59), "0 min");
        assert_eq!(formatar_duracao(30 * 60), "30 min");
        assert_eq!(formatar_duracao(2 * 3600), "2 h");
        assert_eq!(formatar_duracao(90 * 60), "1 h 30 min");
    }

    fn res(status: Status) -> Resultado {
        Resultado {
            id: "api".into(),
            nome: "API".into(),
            grupo: "G".into(),
            status,
            latencia_ms: Some(5),
            detalhe: Some("HTTP 502".into()),
        }
    }

    #[test]
    fn ciclo_de_incidente() {
        let dir = std::env::temp_dir().join(format!("nutra-status-ciclo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut dados = Dados::carregar(&dir).unwrap();

        assert!(aplicar(&mut dados, &[res(Status::Online)], t(0)).is_empty());

        let avisos = aplicar(&mut dados, &[res(Status::Offline)], t(1));
        assert_eq!(avisos.len(), 1);
        assert_eq!(avisos[0].evento, Evento::Caiu);
        assert_eq!(dados.incidentes.len(), 1);
        registrar_issue(&mut dados, "api", 42);
        assert_eq!(dados.incidentes[0].issue, Some(42));

        // Ainda fora, antes do intervalo de lembrete: nada a enviar.
        assert!(aplicar(&mut dados, &[res(Status::Offline)], t(2)).is_empty());
        // Ignorado não muda nada.
        assert!(aplicar(&mut dados, &[res(Status::Ignorado)], t(2)).is_empty());

        let avisos = aplicar(&mut dados, &[res(Status::Online)], t(4));
        assert_eq!(
            avisos[0].evento,
            Evento::Voltou {
                duracao_s: 3 * 3600
            }
        );
        assert_eq!(avisos[0].issue, Some(42));
        assert_eq!(dados.incidentes[0].duracao_s, Some(3 * 3600));
        assert_eq!(dados.estado["api"].issue, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
