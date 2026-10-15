//! E-mail via SMTP (mesmas variáveis EMAIL_* da Nutra-API2, Hostinger por padrão).

use std::time::Duration;

use anyhow::{Context, Result};
use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use super::{formatar_duracao, Aviso, Evento};
use crate::model::Status;

pub const DESTINO_PADRAO: &str = "eziochifunga.dev@gmail.com";

pub struct Email {
    host: String,
    porta: u16,
    ssl_implicito: bool,
    usuario: String,
    senha: String,
    remetente: Mailbox,
    destinos: Vec<Mailbox>,
    pagina: Option<String>,
}

impl Email {
    /// Exige EMAIL_USERNAME e EMAIL_PASSWORD. Demais variáveis têm padrão:
    /// EMAIL_HOST=smtp.hostinger.com, EMAIL_PORT=465, EMAIL_FROM=EMAIL_USERNAME,
    /// EMAIL_FROM_NAME="Nutra Status", ALERT_EMAIL_TO=eziochifunga.dev@gmail.com (vírgula separa vários).
    pub fn do_ambiente() -> Result<Option<Self>> {
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.trim().is_empty());
        let (Some(usuario), Some(senha)) = (var("EMAIL_USERNAME"), var("EMAIL_PASSWORD")) else {
            return Ok(None);
        };
        let host = var("EMAIL_HOST").unwrap_or_else(|| "smtp.hostinger.com".into());
        let porta: u16 = var("EMAIL_PORT")
            .map(|p| p.parse().context("EMAIL_PORT inválida"))
            .transpose()?
            .unwrap_or(465);
        let ssl_implicito = var("EMAIL_USE_SSL")
            .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
            .unwrap_or(porta == 465);
        let nome = var("EMAIL_FROM_NAME").unwrap_or_else(|| "Nutra Status".into());
        let endereco = var("EMAIL_FROM").unwrap_or_else(|| usuario.clone());
        let remetente = Mailbox::new(Some(nome), endereco.parse().context("EMAIL_FROM inválido")?);
        let destinos = var("ALERT_EMAIL_TO")
            .unwrap_or_else(|| DESTINO_PADRAO.into())
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse()
                    .with_context(|| format!("ALERT_EMAIL_TO inválido: {s}"))
            })
            .collect::<Result<Vec<Mailbox>>>()?;
        Ok(Some(Self {
            host,
            porta,
            ssl_implicito,
            usuario,
            senha,
            remetente,
            destinos,
            pagina: var("STATUS_PAGE_URL"),
        }))
    }

    pub async fn enviar(&self, avisos: &[&Aviso]) -> Result<()> {
        let (assunto, corpo) = montar(avisos, self.pagina.as_deref());
        let mut builder = Message::builder()
            .from(self.remetente.clone())
            .subject(assunto)
            .header(ContentType::TEXT_PLAIN);
        for destino in &self.destinos {
            builder = builder.to(destino.clone());
        }
        let mensagem = builder.body(corpo)?;

        let transporte = if self.ssl_implicito {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&self.host)?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host)?
        }
        .port(self.porta)
        .credentials(Credentials::new(self.usuario.clone(), self.senha.clone()))
        .timeout(Some(Duration::from_secs(20)))
        .build();

        transporte.send(mensagem).await.context("SMTP")?;
        println!(
            "✉️  Alerta enviado para {} destinatário(s)",
            self.destinos.len()
        );
        Ok(())
    }
}

/// Monta assunto e corpo (texto puro) de um e-mail que agrupa todos os avisos da rodada.
pub fn montar(avisos: &[&Aviso], pagina: Option<&str>) -> (String, String) {
    let caidos: Vec<&&Aviso> = avisos
        .iter()
        .filter(|a| !matches!(a.evento, Evento::Voltou { .. }))
        .collect();
    let voltaram = avisos.len() - caidos.len();

    let assunto = match (caidos.len(), voltaram) {
        (1, 0) => format!(
            "🔴 [Nutra Status] {} {}",
            caidos[0].nome,
            rotulo(caidos[0].status)
        ),
        (n, 0) => format!("🔴 [Nutra Status] {n} serviços com problema"),
        (0, 1) => format!("✅ [Nutra Status] {} voltou", avisos[0].nome),
        (0, n) => format!("✅ [Nutra Status] {n} serviços voltaram"),
        (n, m) => format!("🔴 [Nutra Status] {n} com problema, {m} recuperado(s)"),
    };

    let mut corpo = String::new();
    for a in avisos {
        let linha = match a.evento {
            Evento::Voltou { duracao_s } => format!(
                "✅ {} ({}) voltou. Ficou fora por {}.",
                a.nome,
                a.id,
                formatar_duracao(duracao_s)
            ),
            _ => format!(
                "{} {} ({}) está {} desde {} UTC.",
                if a.status == Status::Degradado {
                    "🟡"
                } else {
                    "🔴"
                },
                a.nome,
                a.id,
                a.status.rotulo(),
                a.desde.format("%d/%m/%Y %H:%M")
            ),
        };
        corpo.push_str(&linha);
        corpo.push('\n');
        if let Some(d) = a.detalhe.as_deref().filter(|_| a.status.com_problema()) {
            corpo.push_str(&format!("   Detalhe: {d}\n"));
        }
        corpo.push('\n');
    }
    if let Some(p) = pagina {
        corpo.push_str(&format!("Página de status: {p}\n"));
    }
    corpo.push_str("\n— Nutra Status (GitHub Actions)\n");
    (assunto, corpo)
}

fn rotulo(status: Status) -> &'static str {
    match status {
        Status::Degradado => "com instabilidade",
        _ => "fora do ar",
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use chrono::Utc;

    fn aviso(id: &str, status: Status, evento: Evento) -> Aviso {
        Aviso {
            id: id.into(),
            nome: id.to_uppercase(),
            status,
            evento,
            detalhe: Some("HTTP 502".into()),
            desde: Utc::now(),
            issue: None,
        }
    }

    #[test]
    fn assuntos() {
        let caiu = aviso("api", Status::Offline, Evento::Caiu);
        let voltou = aviso("engine", Status::Online, Evento::Voltou { duracao_s: 3600 });
        let (assunto, corpo) = montar(&[&caiu], None);
        assert_eq!(assunto, "🔴 [Nutra Status] API fora do ar");
        assert!(corpo.contains("Detalhe: HTTP 502"));
        let (assunto, corpo) = montar(&[&voltou], Some("https://x"));
        assert_eq!(assunto, "✅ [Nutra Status] ENGINE voltou");
        assert!(corpo.contains("1 h"));
        assert!(!corpo.contains("Detalhe"));
        let (assunto, _) = montar(&[&caiu, &voltou], None);
        assert_eq!(assunto, "🔴 [Nutra Status] 1 com problema, 1 recuperado(s)");
    }
}
