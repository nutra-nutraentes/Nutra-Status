mod alerts;
mod checks;
mod config;
mod model;
mod runner;
mod store;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Utc;

use crate::model::{Resultado, Status};

const AJUDA: &str = "\
nutra-status — checa os serviços da Nutra, grava o histórico e envia alertas

USO:
    nutra-status [OPÇÕES]

OPÇÕES:
    --config <arquivo>    Lista de checagens (padrão: config/checks.toml)
    --data-dir <pasta>    Onde ler/gravar os JSONs (padrão: data)
    --only <id>[,<id>]    Executa só estas checagens (não grava nada)
    --dry-run             Não envia e-mail nem mexe em issues (grava os dados)
    --verbose             Mostra o detalhe dos erros no terminal
    -h, --help            Mostra esta ajuda

No GitHub Actions o detalhe nunca é impresso, pois os logs de repositório público são públicos.
";

struct Args {
    config: PathBuf,
    data_dir: PathBuf,
    only: Option<Vec<String>>,
    dry_run: bool,
    verbose: bool,
}

fn ler_args() -> Result<Args> {
    let mut args = Args {
        config: PathBuf::from("config/checks.toml"),
        data_dir: PathBuf::from("data"),
        only: None,
        dry_run: false,
        verbose: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => args.config = it.next().context("--config sem valor")?.into(),
            "--data-dir" => args.data_dir = it.next().context("--data-dir sem valor")?.into(),
            "--only" => {
                let lista = it.next().context("--only sem valor")?;
                args.only = Some(lista.split(',').map(|s| s.trim().to_string()).collect());
            }
            "--dry-run" => args.dry_run = true,
            "--verbose" | "-v" => args.verbose = true,
            "-h" | "--help" => {
                print!("{AJUDA}");
                std::process::exit(0);
            }
            outro => bail!("argumento desconhecido: {outro}\n\n{AJUDA}"),
        }
    }
    Ok(args)
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let args = ler_args()?;
    let em_ci = std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true");
    let mostrar_detalhe = args.verbose && !em_ci;

    let mut defs = config::carregar(&args.config)?;
    if let Some(only) = &args.only {
        defs.retain(|d| only.contains(&d.id));
        if defs.is_empty() {
            bail!("nenhuma checagem com id em {only:?}");
        }
    }

    let ctx = checks::Contexto::novo(Duration::from_secs(10))?;
    let opcoes = runner::Opcoes {
        timeout: Duration::from_secs(25),
        espera_retentativa: Duration::from_secs(15),
    };
    let agora = Utc::now();
    let resultados = runner::executar_todos(&defs, &ctx, &opcoes).await;

    imprimir(&resultados, mostrar_detalhe);
    escrever_resumo_actions(&resultados);

    if args.only.is_some() {
        println!("\n--only: nada foi gravado nem notificado.");
        return Ok(());
    }

    let mut dados = store::Dados::carregar(&args.data_dir)?;
    let avisos = alerts::aplicar(&mut dados, &resultados, agora);

    if args.dry_run {
        for a in &avisos {
            println!("(dry-run) alerta {:?} para {}", a.evento, a.id);
        }
    } else {
        let github = alerts::GitHub::do_ambiente(ctx.http.clone());
        let email = match alerts::Email::do_ambiente() {
            Ok(e) => e,
            Err(erro) => {
                eprintln!("⚠️  E-mail: configuração inválida: {erro:#}");
                None
            }
        };
        alerts::notificar(&avisos, &mut dados, github.as_ref(), email.as_ref()).await;
    }

    dados.salvar(&resultados, agora)?;
    println!(
        "\nDados gravados em {} ({} alerta(s))",
        args.data_dir.display(),
        avisos.len()
    );
    Ok(())
}

fn icone(status: Status) -> &'static str {
    match status {
        Status::Online => "🟢",
        Status::Degradado => "🟡",
        Status::Offline => "🔴",
        Status::Ignorado => "⚪",
    }
}

fn imprimir(resultados: &[Resultado], detalhe: bool) {
    let largura = resultados.iter().map(|r| r.id.len()).max().unwrap_or(4);
    for r in resultados {
        let ms = r
            .latencia_ms
            .map(|m| format!("{m} ms"))
            .unwrap_or_else(|| "-".into());
        print!(
            "{} {:<largura$}  {:<9}  {:>8}",
            icone(r.status),
            r.id,
            r.status.rotulo(),
            ms
        );
        // "ignorado" só diz qual secret falta: seguro para imprimir sempre.
        match &r.detalhe {
            Some(d) if detalhe || r.status == Status::Ignorado => println!("  {d}"),
            _ => println!(),
        }
    }
    println!("\nGeral: {}", store::status_geral(resultados).rotulo());
}

fn escrever_resumo_actions(resultados: &[Resultado]) {
    let Ok(caminho) = std::env::var("GITHUB_STEP_SUMMARY") else {
        return;
    };
    let mut md = String::from("| | Serviço | Status | Latência |\n|---|---|---|---|\n");
    for r in resultados {
        md.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            icone(r.status),
            r.nome,
            r.status.rotulo(),
            r.latencia_ms
                .map(|m| format!("{m} ms"))
                .unwrap_or_else(|| "-".into())
        ));
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(caminho)
    {
        let _ = f.write_all(md.as_bytes());
    }
}
