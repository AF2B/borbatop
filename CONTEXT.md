# CONTEXT.md

## Visão geral do domínio

`borbatop` é um gerenciador de tarefas nativo para Linux, em modo texto (TUI), escrito em Rust e pensado para Arch e derivadas. O "domínio" é o próprio sistema operacional: processos, CPU, memória, pressão de recursos (PSI), rede, disco, serviços systemd e o estado do gerenciador de pacotes (pacman). Tudo é lido direto do kernel (`/proc`, `/sys`, cgroup v2) e de poucos comandos do sistema, sem daemon.

A regra de ouro é "núcleo puro, efeitos nas bordas": tudo que interpreta texto ou calcula números vive num crate sem I/O e sem relógio; tudo que toca o sistema vive em um crate separado; a tela só recebe dados prontos.

## Bounded Contexts

### Processos
- Responsabilidade: identificar cada processo (PID + hora de início), medir uso de CPU por delta de contadores e resolver dono, unit systemd, pacote e linha de comando.
- Namespaces principais: `borbatop_core::process` (`ProcessSample`, `ProcessId`, `parse_process_stat`), `borbatop_core::cpu` (`cpu_permille`, `elapsed_per_core`), `borbatop_core::identity` (`UserTable`, `parse_cgroup_unit`), `borbatop_sources::proc` (`ProcCollector`), `borbatop_sources::statics` (`ProcessStatics`), `borbatop_sources::owners` (`FileOwners`), `borbatop_sources::signal` (`send_signal`).
- Contratos de tipo: `ProcessId` (a chave que liga duas amostras; PID sozinho é proibido para isso), `Permille` (uso de CPU sem `f64`), `SamplePair<T>` (evita trocar anterior e atual), `StatParseError`, `SampleError`, `SignalError`.

### Sistema (CPU, memória, pressão, temperatura)
- Responsabilidade: ler e interpretar contadores globais da máquina.
- Namespaces principais: `borbatop_core::memory` (`parse_meminfo`, `parse_loadavg`, `parse_uptime`, `parse_pressure`, `parse_millicelsius`), `borbatop_core::decimal` (`Hundredths`), `borbatop_core::bytes` (`Bytes`, `PageSize`), `borbatop_sources::system` (`SystemCollector`, `SystemSnapshot`).
- Contratos de tipo: `MemInfo`, `Pressure`, `LoadAvg`, `MilliCelsius`, `Reading<T>` (leitura opcional que pode falhar sem derrubar a amostra).

### Rede e disco
- Responsabilidade: contadores cumulativos, vazão por delta e uso dos sistemas de arquivos.
- Namespaces principais: `borbatop_core::io_stats` (`parse_net_dev`, `parse_diskstats`, `parse_mounts`, `bytes_per_second`), `borbatop_sources::system` (`MountReport`, `MountUsage`).
- Contratos de tipo: `NetCounters`, `DiskCounters`, `Mount`, `BytesPerSecond`, `IoStatsError`.

### Serviços (systemd)
- Responsabilidade: listar units e ler o journal de uma unit.
- Namespaces principais: `borbatop_core::services` (`parse_systemctl_units`, `ActiveState`, `UnitRow`), `borbatop_sources::units` (`list_units`, `unit_logs`).
- Contratos de tipo: `ActiveState` (enum fechado), `CommandError::InvalidUnitName` (nome de unit validado antes de chegar ao `journalctl`).

### Pacman
- Responsabilidade: atualizações pendentes, órfãos, kernel em execução versus instalado, `.pacnew`/`.pacsave`, bibliotecas substituídas e dono de cada executável.
- Namespaces principais: `borbatop_core::services` (`parse_upgrades`, `package_files`, `package_name_from_dir`, `reboot_recommended`, `deleted_libraries`, `is_pending_config`), `borbatop_sources::pacman` (`collect_pacman_report`, `PacmanReport`), `borbatop_sources::owners`.
- Contratos de tipo: `PacmanReport` (cada parte com seu próprio `Reading`), `KernelStatus`, `PackageLineError`.

### Tela
- Responsabilidade: estado da interface, teclado, desenho. Sem terminal, sem I/O.
- Namespaces principais: `borbatop_tui::app` (`App`, `handle_key`), `borbatop_tui::draw` (`draw`, `WORDMARK`), `borbatop_tui::model` (`Model` e linhas), `borbatop_tui::input` (`Key`, `Command`), `borbatop_tui::terminal` (`key_from_event`), `borbatop_tui::theme`.
- Contratos de tipo: `Key` e `Command` (a tela devolve intenções, não executa efeitos), enums de estado (`Playback`, `InputMode`, `ProcessLayout`, `Visibility`, `PacmanState`) no lugar de booleanos.

### Aplicação
- Responsabilidade: ligar as peças, threads, configuração, CLI.
- Namespaces principais: `borbatop::main`, `borbatop::run`, `borbatop::worker`, `borbatop::model` (`ModelBuilder`), `borbatop::config`, `borbatop::cli`, `borbatop::convert`.
- Contratos de tipo: `Settings` (validado uma vez), `FileConfig` (`deny_unknown_fields`), `WorkerEvent`.

## Arquitetura e Camadas

- `borbatop-core` (puro): tipos de domínio, parsers de texto, matemática de deltas, ordenação em árvore, histórico limitado. Sem dependências em runtime; sem I/O; sem relógio.
- `borbatop-sources` (efeitos): leitura de `/proc`, `/sys`, `/etc/passwd`, banco local do pacman; execução de `systemctl`, `journalctl`, `pacman`; `statvfs`, `pidfd` e tamanho de página via `rustix` (wrappers seguros). Converte tudo em tipos do `core`.
- `borbatop-tui` (apresentação): `ratatui`. Recebe um `Model` imutável e devolve `Command`s. Não conhece `sources`.
- `borbatop` (borda): threads, canais, terminal (`ratatui::try_init`), CLI (`clap`), arquivo TOML (`serde` + `toml`), completions e man page.

Regra de dependência: `borbatop → {sources, tui} → core`, nunca o inverso; `sources` e `tui` não dependem um do outro. `unsafe_code = "forbid"` no workspace inteiro.

## Fluxo de Dados

1. A thread amostradora (`worker::sample_forever`) chama `SystemCollector::collect` a cada `interval`.
2. `ProcCollector` lê `/proc/stat` e `/proc/[pid]/stat` de cada processo, reaproveitando buffers. Fatos imutáveis (usuário, unit, pacote, linha de comando) são lidos uma vez por `ProcessId` e guardados em cache (`StaticsResolver`). Processo que some no meio da leitura vira `vanished`; qualquer outro problema vira `failures`.
3. `SystemCollector` acrescenta memória, carga, uptime, PSI, temperatura, rede, discos e montagens. Cada leitura opcional vem como `Reading<T>`.
4. `ModelBuilder::build` compara com a amostra anterior (chave `ProcessId`) e produz o `Model`: CPU por delta, memória em bytes, vazão de rede e disco, históricos para sparklines.
5. O `Model` chega à thread da interface por `mpsc` (`WorkerEvent::Model`); `App::apply_model` o guarda (ou ignora, se pausado).
6. A cada 100 ms, `run::event_loop` desenha (`draw`) e lê o teclado (`key_from_event` → `Key` → `App::handle_key` → `Option<Command>`).
7. `Command` é executado em `run::execute`: sinal via `send_signal` (pidfd + conferência da hora de início), relatório do pacman e journal em threads próprias que respondem por `WorkerEvent`.

## Dependências Externas

- Kernel Linux: `/proc`, `/sys`, cgroup v2, PSI (`/proc/pressure`), `pidfd_open` (kernel 5.3 ou superior; `<a confirmar>` se algum caminho precisa de fallback).
- `systemctl` e `journalctl`: usados pela camada `sources::units`, sob demanda (lista a cada 5 amostras; journal por tecla).
- `pacman`: `pacman -Qu` e `pacman -Qdtq` (código de saída 1 significa lista vazia); banco local em `/var/lib/pacman/local` lido diretamente, sem `libalpm`.
- Crates: `rustix` (syscalls seguras), `ratatui` (tela), `clap`, `clap_complete`, `clap_mangen` (CLI), `serde`, `toml` (config). Apenas em testes: `criterion`, `proptest`.

## Decisões Relevantes

- Threads e canais da `std`, sem runtime async: a leitura de procfs é bloqueante e rápida.
- `systemctl` por processo filho em vez de D-Bus (`zbus`): sem dependência nova, ao custo de um processo por consulta.
- Banco do pacman lido à mão em vez de `alpm`: evita linkar `libalpm` e depender da versão do pacman.
- Uso de CPU em permille inteiro (`Permille`), decimais em centésimos (`Hundredths`): nenhuma aritmética de ponto flutuante (`float_arithmetic = "deny"`).
- A identidade de um processo é PID mais hora de início. O envio de sinal confere isso depois de fixar o processo com um pidfd, para nunca atingir um PID reaproveitado.
- Dados vindos de fora são tratados como não confiáveis: arquivos do procfs são decodificados com UTF-8 tolerante (o kernel corta `comm` em 15 bytes e pode partir um caractere); caracteres de controle são trocados antes de desenhar; `Debug` de `ProcessStatics` e `ProcessRow` esconde a linha de comando.
- Testes que dependem do host (systemd, módulos do kernel, raiz em dispositivo de bloco) saem cedo quando a máquina não os tem, para rodar em contêiner.
- `#[expect(clippy::wildcard_enum_match_arm)]` só em `key_from_event`, porque `KeyCode` do crossterm tem muitas teclas ignoradas de propósito.
- `<a confirmar>`: URL do repositório, publicação no AUR e primeira execução real do workflow de CI (nenhum dos dois ocorreu).
