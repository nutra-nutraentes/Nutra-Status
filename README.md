# Nutra Status

Monitor de disponibilidade da plataforma Nutra. Um binário em Rust roda no
**GitHub Actions a cada 30 minutos** e checa a API, a API Web, a Engine, os
domínios, os bancos de dados, o Arnut e a Cloudflare. Ele também:

- grava o histórico de checagens na branch `status-data` deste repositório;
- publica uma **página de status** estática no GitHub Pages;
- avisa quando algo cai ou volta, **por e-mail** e por uma **issue** no GitHub.
  A issue chega como push no app do GitHub.

Este repositório é independente da Nutra-API2: se a API cair, o monitor continua de pé.

## O que é checado

A lista fica em [`config/checks.toml`](config/checks.toml).

| Grupo | Checagens |
|---|---|
| Plataforma | API (`/health`), API Web (`/health`, exige `banco: true`), Engine (`/health`), Engine via API, Arnut (`/info`), n8n, EasyPanel |
| Domínios | registro **A** + HTTP de `nutraentes.com.br`, `workspace.`, `admin.`, `kids.`; registro A de `api.` e `api-web.` |
| Bancos de dados | Postgres (`SELECT 1`), Firebase Auth (lista 1 usuário com a service account), Qdrant (`/readyz`) |
| Infra externa | Status global da Cloudflare (cloudflarestatus.com), token da Cloudflare válido e ativo (`/user/tokens/verify`) |

Cada checagem termina em um destes status:

- `online`;
- `degradado`, por exemplo quando a Cloudflare reporta instabilidade (`minor` ou `major`);
- `offline`;
- `ignorado`, quando o secret necessário não foi configurado. Nesse caso não há alerta nem contagem de uptime.

Uma checagem com problema é **repetida uma vez após 15 s** antes de ser considerada fora do ar, para evitar falso alarme.

> `app.nutraentes.com.br` está desativado (`ativo = false`) porque não tinha registro A em 15/10/2026.

## Alertas

Os alertas são enviados **só na mudança de estado**, nunca a cada rodada.

| Evento | E-mail | Issue |
|---|---|---|
| Serviço caiu | ✅ (um e-mail por rodada, agrupando tudo) | abre `🔴 <serviço> fora do ar` (label `incidente`) |
| Piorou (degradado → offline) | ✅ | comentário |
| Continua fora | — | lembrete a cada 6 h |
| Voltou | ✅ com a duração da queda | comentário + fecha a issue |

- O e-mail usa o SMTP da Hostinger, com as mesmas variáveis `EMAIL_*` da Nutra-API2.
- O destino padrão é `eziochifunga.dev@gmail.com`. Para mudar, defina `ALERT_EMAIL_TO`, separando vários endereços por vírgula.
- Se o envio do e-mail falhar, a issue continua funcionando como canal de alerta.
- Se o próprio job falhar, o GitHub envia o aviso padrão de workflow com falha.

> **Privacidade:** este repositório é público. Página, histórico, issues e logs do
> Actions mostram só nome, status e latência. IPs, hosts e mensagens de erro (`detalhe`)
> aparecem **apenas no e-mail**.

## Configuração (uma vez)

1. **Pages:** em *Settings → Pages → Build and deployment → Source*, escolha **GitHub Actions**.
2. **Secrets:** em *Settings → Environments → `.env` → Environment secrets*, cadastre os que forem usados. O job `checar` usa esse environment, e secrets cadastrados em outro lugar não chegam a ele. Os valores são os mesmos do `.env` da Nutra-API2.

   | Secret | Uso |
   |---|---|
   | `ENGINE_URL`, `ARNUT_URL`, `N8N_URL`, `EASYPANEL_URL`, `QDRANT_URL` | alvos privados |
   | `DB_HOST`, `DB_PORT`, `DB_NAME`, `DB_USERNAME`, `DB_PASSWORD`, `DB_SSL`, `DB_SSL_CA` | Postgres |
   | `FIREBASE_SERVICE_ACCOUNT_JSON` | Firebase Auth |
   | `CLOUDFLARE_API_TOKEN` | token da Cloudflare (`CLOUDFLARE_ACCOUNT_ID` não é mais necessário) |
   | `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_USERNAME`, `EMAIL_PASSWORD`, `EMAIL_FROM`, `EMAIL_FROM_NAME`, `EMAIL_USE_SSL` | SMTP |
   | `ALERT_EMAIL_TO` | destinatários (opcional) |

3. **Variables** (em *Environment variables*, no mesmo environment `.env`; ambas opcionais):
   - `ALERT_GITHUB_USER`: seu usuário do GitHub, mencionado nas issues.
   - `STATUS_PAGE_URL`: endereço da página, se for usado um domínio próprio.
4. Rode o workflow **Status** manualmente (*Actions → Status → Run workflow*). A primeira execução cria a branch `status-data` e publica a página.

`GITHUB_TOKEN` é fornecido pelo Actions; não precisa cadastrar.

### Postgres e firewall

Os runners do GitHub usam IPs variáveis. Se o firewall da VPS bloquear conexões
externas na porta do Postgres ou do Qdrant, essas checagens vão acusar `offline`.
Nesse caso, há duas saídas:

- liberar a porta, preferencialmente com TLS e um usuário só leitura;
- desativar a checagem no `checks.toml` com `ativo = false`. O check `api-web` já valida o banco de forma indireta.

## Como funciona

```
GitHub Actions (cron */30)
  └─ cargo run --release -- --data-dir state/data
       ├─ executa os checks em paralelo (timeout 25 s, 1 retentativa)
       ├─ compara com state.json → eventos (caiu / piorou / lembrete / voltou)
       ├─ envia e-mail (SMTP) e abre/comenta/fecha issues (API do GitHub)
       └─ grava data/*.json
  ├─ commit + push na branch status-data
  └─ site/ + data/ → GitHub Pages
```

Arquivos em `data/` (branch `status-data`):

| Arquivo | Conteúdo |
|---|---|
| `current.json` | última rodada: status geral e de cada serviço |
| `history/<id>.json` | últimas 1440 amostras (~30 dias) `{t, s, ms}` |
| `daily.json` | agregado diário dos últimos 90 dias (`checks`, `falhas`, `degradados`, `uptime_pct`) |
| `incidents.json` | últimas 200 quedas, com início, fim, duração e número da issue |
| `state.json` | estado anterior de cada serviço (usado para detectar mudanças) |

## Rodando localmente

```bash
cp .env.example .env          # preencha o que quiser testar
cargo run -- --dry-run --verbose --data-dir site/data
python -m http.server -d site 8000   # abra http://localhost:8000
```

| Opção | Efeito |
|---|---|
| `--dry-run` | não envia e-mail nem mexe em issues (mas grava os dados) |
| `--verbose` | mostra o detalhe dos erros no terminal (nunca no Actions) |
| `--only api,engine` | roda só essas checagens e não grava nada |
| `--config`, `--data-dir` | caminhos alternativos |

Testes: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.

> No Windows, se o build falhar com erros estranhos de metadata da `std`, use `cargo build -j1`.

## Adicionando uma checagem

Acrescente um bloco em `config/checks.toml`:

```toml
[[check]]
id = "novo-servico"          # único; vira nome de arquivo
nome = "Novo serviço"
grupo = "Plataforma"         # Plataforma | Domínios | Bancos de dados | Infra externa
tipo = "http"
url = "${NOVO_SERVICO_URL}/health"   # ${VAR} vem dos secrets
status_ok_ate = 299
esperar_json = { status = "ok" }
```

Se usar uma variável nova, cadastre o secret e repasse-o no `env:` do passo
*Rodar checagens* em [`.github/workflows/status.yml`](.github/workflows/status.yml).

## Observações

- O cron do GitHub é *best effort*: as rodadas podem atrasar alguns minutos.
- O GitHub desativa workflows agendados após 60 dias sem atividade no repositório.
  Os commits na `status-data` contam como atividade; se o workflow for desativado,
  reative-o em *Actions*.
