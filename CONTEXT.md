# CONTEXT.md

## Visão geral do domínio

`borbatop` é um gerenciador de tarefas nativo para Linux, em modo texto (TUI), escrito em Rust e pensado para Arch e derivadas. O "domínio" é o próprio sistema operacional: processos, CPU, memória, pressão de recursos (PSI), rede, disco, serviços systemd e o estado do gerenciador de pacotes (pacman). Tudo é lido direto do kernel (`/proc`, `/sys`, cgroup v2) e de poucos comandos do sistema, sem daemon.

A regra de ouro é "núcleo puro, efeitos nas bordas": tudo que interpreta texto ou calcula números vive num crate sem I/O e sem relógio; tudo que toca o sistema vive em um crate separado; a tela só recebe dados prontos.

## Bounded Contexts

### Processos
- Responsabilidade: identificar cada processo (PID + hora de início), medir uso de CPU por delta de contadores e resolver dono, unit systemd, pacote e linha de comando.
- Namespaces principais: `borbatop_core::process` (`ProcessSample`, `ProcessId`, `Nice`, `parse_process_stat`), `borbatop_core::cpu` (`cpu_permille`, `elapsed_per_core`), `borbatop_core::identity` (`UserTable`, `parse_cgroup_unit`), `borbatop_core::proc_detail` (`classify_fd_target`, `parse_limits`, `status_size`), `borbatop_sources::proc` (`ProcCollector`), `borbatop_sources::statics` (`ProcessStatics`), `borbatop_sources::owners` (`FileOwners`), `borbatop_sources::signal` (`send_signal`), `borbatop_sources::schedule` (`set_nice`, `set_affinity`), `borbatop_sources::detail` (`process_detail`), `borbatop_sources::identity` (`current_identity`, interno).
- Contratos de tipo: `ProcessId` (a chave que liga duas amostras; PID sozinho é proibido para isso), `Permille` (uso de CPU sem `f64`), `SamplePair<T>` (evita trocar anterior e atual), `Nice` (limitado a -20..=19), `StatParseError`, `SampleError`, `SignalError`, `ScheduleError`. `ProcessDetail` e `OpenDescriptor` escondem caminhos no `Debug`.

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

### Conexões e diagnóstico
- Responsabilidade: sockets com dono (`/proc/net` + `/proc/[pid]/fd`), tempo de boot, journal do sistema, GPUs e containers. Cada um vira um `Report` (linhas prontas) carregado sob demanda.
- Namespaces principais: `borbatop_core::sockets` (`parse_socket_table`, `parse_socket_link`), `borbatop_core::startup` (`parse_startup_blame`, `parse_systemd_duration`), `borbatop_core::gpu` (`GpuCard`, `parse_nvidia_smi`), `borbatop_core::containers` (`container_id_of_unit`, `parse_container_list`), `borbatop_sources::{connections, diagnostics, gpu, containers}`, `borbatop::reports` (formatação pura de cada aba).
- Contratos de tipo: `SocketState` (enum fechado), `SocketProtocol`, `GpuVendor`, `Report`/`ReportLine`/`ReportTone` (o que a tela recebe).

### Alertas
- Responsabilidade: apontar leituras acima do limite (CPU, memória, swap, PSI, temperatura, units falhas) no cabeçalho de qualquer aba e anunciar cada alerta novo uma vez.
- Namespaces principais: `borbatop_tui::alerts` (`evaluate`, `AlertThresholds`, `AlertKind`); limites vêm da tabela `[alerts]` do TOML.

### Containers e atualização de pacotes
- Responsabilidade: listar containers (rodando ou parados) de cada runtime instalado e agir sobre eles; descobrir as fontes de atualização (repositórios, cada helper do AUR) e montar o comando de atualização.
- Namespaces principais: `borbatop_core::containers` (`ContainerInfo`, `container_id_of_unit`), `borbatop_sources::containers` (`list_containers`, `control_container`, `container_logs`, nomes validados), `borbatop_sources::pacman` (`UpgradeSource`, `AurUpgrades` por helper), `borbatop_sources::updater` (`update_command`: valida e devolve o comando, não executa), `borbatop_tui::dialog` (`UpdateDialog`, `PackagesDialog`, `ContainerDialog`).
- Contratos de tipo: `ContainersView`/`ContainerView` (o que a aba recebe), `UpdateRequest`/`UpdateCommand`, `RepoSource`. O comando de atualização roda no terminal real: `run::update_on_terminal` desfaz a tela, executa, espera Enter e refaz a tela.

### Exportação
- Responsabilidade: gravar a aba atual em JSON do jeito que a pessoa a vê (filtro, ordem, linhas), sem linhas de comando.
- Namespaces principais: `borbatop_tui::input` (`ExportRequest`, `ExportContent`), `borbatop::export` (`write_export`, `export_json`), `borbatop_core::clock` (`UtcDateTime`, nome do arquivo em UTC).

### Gravação e replay
- Responsabilidade: gravar amostras em JSON Lines e reproduzi-las na tela.
- Namespaces principais: `borbatop_core::json` (`Json`, leitor e escritor sem dependência), `borbatop::snapshot` (`model_json`, `LiveSampler`), `borbatop::recording` (`record`, `load`), `borbatop::run` (`replay`).
- Contratos de tipo: `Json`/`JsonError`, `RecordingError` (arquivo, linha e campo do problema). O arquivo é criado com modo 0600 e nunca sobrescreve outro.

### Tela
- Responsabilidade: estado da interface, teclado, mouse, diálogos, desenho. Sem terminal, sem I/O.
- Namespaces principais: `borbatop_tui::app` (`App`, `handle_key`, `handle_mouse`), `borbatop_tui::dialog` (sinal, nice, afinidade, serviço), `borbatop_tui::mouse` (`Mouse`, mapa de cliques gravado ao desenhar), `borbatop_tui::draw` (`draw`, `WORDMARK`), `borbatop_tui::model` (`Model` e linhas, `Report`), `borbatop_tui::input` (`Key`, `Command`, `Tab`), `borbatop_tui::terminal` (`key_from_event`, `mouse_from_event`), `borbatop_tui::theme`.
- Contratos de tipo: `Key`, `Mouse` e `Command` (a tela devolve intenções, não executa efeitos), enums de estado (`Playback`, `InputMode`, `ProcessLayout`, `Visibility`, `PacmanState`, `Fetch<T>`, `Dialog`) no lugar de booleanos.

### Aplicação
- Responsabilidade: ligar as peças, threads, configuração, CLI.
- Namespaces principais: `borbatop::main`, `borbatop::run`, `borbatop::worker`, `borbatop::model` (`ModelBuilder`), `borbatop::config`, `borbatop::cli`, `borbatop::convert`, `borbatop::reports`, `borbatop::snapshot`, `borbatop::recording`.
- Contratos de tipo: `Settings` (validado uma vez), `FileConfig` (`deny_unknown_fields`), `WorkerEvent`, `CommandLines` (linhas de comando só saem quando pedidas).

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
7. `Command` é roteado em `run::dispatch` e executado em `run::execute`: sinal via `send_signal` (pidfd + conferência da hora de início), renice e afinidade (`schedule`, conferência da hora de início), e tudo o que é lento (relatório do pacman, journal, `systemctl`, detalhe do processo, abas sob demanda) em threads próprias que respondem por `WorkerEvent`.
8. Mouse: `draw` grava em `App::hits` onde pôs abas, títulos de coluna e linhas; `handle_mouse` resolve o clique contra esse mapa.
9. Replay: em vez da thread amostradora, `run::replay` mostra `Model`s carregados de um arquivo (`App::show_model`, que ignora a pausa), no ritmo do intervalo; `Command::SendSignal`, `SetNice`, `SetAffinity`, `ControlUnit` e `LoadDetail` são recusados.

## Dependências Externas

- Kernel Linux: `/proc`, `/sys`, cgroup v2, PSI (`/proc/pressure`), `pidfd_open` (kernel 5.3 ou superior; `<a confirmar>` se algum caminho precisa de fallback).
- `systemctl` e `journalctl`: usados pela camada `sources::units` (lista a cada 5 amostras; journal por tecla; start/stop/restart com `--no-ask-password`) e por `sources::diagnostics` (`systemd-analyze` e journal do sistema, sob demanda).
- Opcionais, sob demanda: `nvidia-smi` (GPU NVIDIA), `podman` e `docker` (aba Containers), `yay` ou `paru` (atualizações do AUR, usam rede).
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
- Nomes de scope de container (`libpod-`, `docker-`, `cri-containerd-`, `crio-` + `.scope`) seguem o driver de cgroup do systemd; `<a confirmar>` contra a documentação de cada runtime antes de depender disso fora do systemd.
- Atualizar pacote isolado no Arch deixa o sistema parcialmente atualizado: os repositórios só têm a ação "atualizar tudo" (`sudo pacman -Syu`); o AUR permite escolher pacotes. A senha nunca passa pelo borbatop: o comando roda no terminal real.
- A lista de upgrades prefere `checkupdates` (lê cópia temporária do banco, lista fresca sem sincronizar o sistema) e cai para `pacman -Qu`, avisando que pode estar desatualizada.
- Depois de uma atualização longa a thread amostradora acumulou amostras: a tela as consome antes de mostrar o resultado, senão a mensagem some na hora.
- Ao ordenar ou inverter, o cursor vai para a primeira linha: a janela segue o cursor, e um cursor no meio da lista escondia o efeito da ordenação.
- Sem crate nova para JSON: `core::json` escreve e lê só o que o programa produz (inteiros, decimais com até duas casas, strings, arrays, objetos). O que não for isso é recusado, não arredondado.
- Renice e afinidade valem por thread no Linux: as funções percorrem `/proc/[pid]/task`. Não existe forma com pidfd; a hora de início é conferida logo antes (risco residual de milissegundos, com efeito só em prioridade ou máscara).
- Processos com capabilities de arquivo (ex.: `kwin_wayland`) escondem `/proc/[pid]/exe` e ficam com pacote `—`; não foi adotado fallback por `argv[0]` porque o próprio processo controla esse texto.
- `<a confirmar>`: publicação no AUR e o resultado da primeira execução do workflow de CI (repositório: github.com/AF2B/borbatop).
