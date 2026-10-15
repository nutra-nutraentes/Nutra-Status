//! Issues no próprio repositório. O repositório é público: nada de `detalhe` aqui.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::{formatar_duracao, Aviso, Evento};

pub struct GitHub {
    http: reqwest::Client,
    api: String,
    token: String,
    repo: String,
    mencao: Option<String>,
    pagina: String,
}

#[derive(Deserialize)]
struct IssueCriada {
    number: u64,
}

impl GitHub {
    /// Usa GITHUB_TOKEN e GITHUB_REPOSITORY (definidos pelo Actions).
    /// ALERT_GITHUB_USER (opcional) é mencionado nas issues para gerar notificação.
    pub fn do_ambiente(http: reqwest::Client) -> Option<Self> {
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.trim().is_empty());
        let token = var("GITHUB_TOKEN")?;
        let repo = var("GITHUB_REPOSITORY")?;
        let api = var("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into());
        let pagina = var("STATUS_PAGE_URL").unwrap_or_else(|| {
            let (dono, nome) = repo.split_once('/').unwrap_or((&repo, ""));
            format!("https://{}.github.io/{}/", dono.to_lowercase(), nome)
        });
        Some(Self {
            http,
            api,
            token,
            repo,
            mencao: var("ALERT_GITHUB_USER").map(|u| u.trim_start_matches('@').to_string()),
            pagina,
        })
    }

    /// Aplica o aviso. Devolve o número da issue quando uma nova é criada.
    pub async fn processar(&self, aviso: &Aviso) -> Result<Option<u64>> {
        match (aviso.evento, aviso.issue) {
            (Evento::Voltou { duracao_s }, Some(n)) => {
                self.comentar(
                    n,
                    &format!(
                        "✅ **{}** voltou a funcionar. Ficou fora por **{}**.",
                        aviso.nome,
                        formatar_duracao(duracao_s)
                    ),
                )
                .await?;
                self.fechar(n).await?;
                Ok(None)
            }
            (Evento::Voltou { .. }, None) => Ok(None),
            (Evento::Caiu, _) | (_, None) => self.criar(aviso).await.map(Some),
            (Evento::Mudou { .. }, Some(n)) => {
                self.comentar(
                    n,
                    &format!(
                        "⚠️ **{}** agora está **{}**.",
                        aviso.nome,
                        aviso.status.rotulo()
                    ),
                )
                .await?;
                Ok(None)
            }
            (Evento::Lembrete, Some(n)) => {
                let fora = (chrono::Utc::now() - aviso.desde).num_seconds();
                self.comentar(
                    n,
                    &format!(
                        "⏰ **{}** continua **{}** há {}.{}",
                        aviso.nome,
                        aviso.status.rotulo(),
                        formatar_duracao(fora),
                        self.cc()
                    ),
                )
                .await?;
                Ok(None)
            }
        }
    }

    fn cc(&self) -> String {
        self.mencao
            .as_ref()
            .map(|u| format!("\n\ncc @{u}"))
            .unwrap_or_default()
    }

    async fn criar(&self, aviso: &Aviso) -> Result<u64> {
        let icone = if aviso.status == crate::model::Status::Degradado {
            "🟡"
        } else {
            "🔴"
        };
        let titulo = format!("{icone} {} {}", aviso.nome, titulo_status(aviso));
        let corpo = format!(
            "**Serviço:** {} (`{}`)\n**Status:** {}\n**Desde:** {} UTC\n\n\
             Detalhes técnicos foram enviados por e-mail. Esta issue é fechada \
             automaticamente quando o serviço voltar.\n\nPágina de status: {}{}",
            aviso.nome,
            aviso.id,
            aviso.status.rotulo(),
            aviso.desde.format("%d/%m/%Y %H:%M"),
            self.pagina,
            self.cc()
        );
        let resposta = self
            .requisicao(
                reqwest::Method::POST,
                &format!("repos/{}/issues", self.repo),
            )
            .json(&json!({ "title": titulo, "body": corpo, "labels": ["incidente"] }))
            .send()
            .await?;
        let status = resposta.status();
        if !status.is_success() {
            bail!(
                "criar issue: HTTP {} {}",
                status,
                resposta.text().await.unwrap_or_default()
            );
        }
        let issue: IssueCriada = resposta.json().await.context("resposta de criar issue")?;
        println!("🐙 Issue #{} aberta para {}", issue.number, aviso.id);
        Ok(issue.number)
    }

    async fn comentar(&self, numero: u64, corpo: &str) -> Result<()> {
        let caminho = format!("repos/{}/issues/{numero}/comments", self.repo);
        let resposta = self
            .requisicao(reqwest::Method::POST, &caminho)
            .json(&json!({ "body": corpo }))
            .send()
            .await?;
        if !resposta.status().is_success() {
            bail!("comentar #{numero}: HTTP {}", resposta.status());
        }
        Ok(())
    }

    async fn fechar(&self, numero: u64) -> Result<()> {
        let caminho = format!("repos/{}/issues/{numero}", self.repo);
        let resposta = self
            .requisicao(reqwest::Method::PATCH, &caminho)
            .json(&json!({ "state": "closed", "state_reason": "completed" }))
            .send()
            .await?;
        if !resposta.status().is_success() {
            bail!("fechar #{numero}: HTTP {}", resposta.status());
        }
        println!("🐙 Issue #{numero} fechada");
        Ok(())
    }

    fn requisicao(&self, metodo: reqwest::Method, caminho: &str) -> reqwest::RequestBuilder {
        self.http
            .request(metodo, format!("{}/{caminho}", self.api))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }
}

fn titulo_status(aviso: &Aviso) -> &'static str {
    match aviso.status {
        crate::model::Status::Degradado => "com instabilidade",
        _ => "fora do ar",
    }
}
