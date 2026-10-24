# Avance

Registro de lo que se va haciendo en hpcd. Lo más reciente arriba.

## Estado actual

**Versión en curso:** v0.3 — Fix (v0.2 — Measure ✅, v0.1 — Audit ✅)

> **Dónde nos quedamos (2026-10-23):** v0.3 partes 1–3 hechas: `hpcd-fix`, `fix --dry-run` y el
> daemon `hpcd` con leases (`fix --rule`, `status`, `commit`, `revert`), probado en WSL. Lo
> siguiente es la parte 4: `fix --interactive`, que mide antes y después de cada fix.

| Pieza | Estado |
|---|---|
| Workspace y crates (`hpcd-core`, `hpcd-probe`, `hpcd-rules`, `hpcctl`, `jitter-demo`) | ✅ |
| Formato de reglas, carga y validación | ✅ |
| Estilo de terminal según `design/hpcctl-terminal.pen` | ✅ |
| `hpcctl explain` (pantalla 02) | ✅ sin evidencia en vivo |
| `hpcctl audit` (pantalla 01) | ✅ las 8 filas |
| `hpcctl audit --snapshot` | ✅ |
| `hpcctl audit --firmware` (pantalla 12) | ✅ máquina, `smt-enabled`, `sub-numa-clustering`, propuesta por fabricante (las que necesitan root quedan para v0.2) |
| `hpcctl capture` (directorio, `--show`) | ✅ |
| CLI instalable (`cargo install`), reglas dentro del binario | ✅ |
| Banner `hpcctl` sin argumentos (pantalla 14) | ✅ |
| `hpcctl measure` con eBPF (aya), p50/p99/p99.9 con intervalo de confianza, `--assert` (pantalla 13) | ✅ probado en WSL |
| Chequeo de estabilidad (pantalla 04) en `measure`, `--wait-stable` | ✅ probado en WSL |
| Métricas de guardrails en `measure`: cpu, power (RAPL), rss, faults, numa misses | ✅ probado en WSL (sin RAPL ni NUMA) |
| Overhead propio en `measure` (CPU de hpcctl + tiempo de eBPF, ns por evento, RSS) | ✅ probado en WSL |
| Detección de SMI: regla `smi-activity` (firmware, HIGH) con `MSR_SMI_COUNT` | ✅ con fixture · sin probar en vivo (WSL no tiene `msr`) |
| Costo de eBPF: sin traducción de PID en el namespace raíz | ✅ (en WSL no aplica) |
| `hpcctl mcp`: 8 herramientas de solo lectura, `--snapshot`, se niega a correr como root | ✅ probado en WSL |
| Set inicial completo: `irq-on-hot-core`, `deep-cstates`, `powersave-governor`, `thread-migration`, `smt-sibling-busy`, `remote-numa-memory`, `isolcpus-missing` | ✅ con fixture |
| Firmware sin root: `smt-enabled`, `sub-numa-clustering` | ✅ |
| Reglas de conflictos: `irqbalance-active`, `tuned-active` | ✅ |
| v0.3 parte 1: `hpcd-fix` (cambios con snapshot, journal, verificación, rollback, recuperación) | ✅ probado en WSL |
| v0.3 parte 2: `hpcctl fix --dry-run` (planificadores por acción, conflictos activos) | ✅ probado en WSL |
| v0.3 parte 3: daemon `hpcd` (socket, `SO_PEERCRED`, policy, leases) y `fix --rule`, `status`, `commit`, `revert` | ✅ probado en WSL |
| v0.3 parte 4: `fix --interactive` | ❌ siguiente |
| v0.3 partes 5–6: MCP de acción, `hpcctl run` | ❌ |
| Reglas de conflictos restantes (`systemd-cpuaffinity`, `cpuset-restricted`, `k8s-cpu-manager`) | ❌ |

## Cómo probar

```powershell
cargo test --workspace
cargo run -p hpcctl
cargo run -p hpcctl -- explain irq-on-hot-core
cargo run -p hpcctl -- audit --unit order-gateway.service --snapshot rules/tests/order-gateway
cargo run --release -p jitter-demo -- --noise
```

Medir latencia (Linux/WSL, necesita nightly + `bpf-linker`, ver README):

```bash
export CARGO_TARGET_DIR=~/hpcd-target
cargo install --locked --path crates/hpcctl
~/hpcd-target/release/jitter-demo --noise > /dev/null &
sudo ~/.cargo/bin/hpcctl measure --pid $! --windows 3 --window 5 --assert "p99 < 1ms"
kill %1
```

`audit` en vivo solo funciona en Linux. En Windows usar `--snapshot`.

Como CLI instalada:

```powershell
cargo install --path crates/hpcctl
hpcctl audit --unit order-gateway.service --snapshot rules/tests/order-gateway
```

## Registro

### 2026-10-23 · v0.3 parte 3: el daemon `hpcd`

- Nuevo crate `hpcd-daemon` (binario `hpcd`), el único que enlaza `hpcd-fix`.
- Protocolo: JSON por líneas sobre `/run/hpcd/hpcd.sock` (`hpcd_core::ipc`), una petición por
  conexión. El README decía bincode; se cambió porque `bincode` está declarado sin mantenimiento
  (aviso de RustSec, 2025) y `serde_json` ya estaba.
- El cliente pide una regla sobre un objetivo, nunca cambios: el daemon observa, evalúa y
  planifica. Así un cliente autorizado no puede mandar cambios arbitrarios.
- Quién puede actuar (`SO_PEERCRED`): root, el usuario del daemon y los miembros de los grupos de
  `[access]` en `policy.toml` (por defecto `hpcd`). Cualquiera puede ver el estado; commit y
  revert solo el dueño del lease o root. `policy.toml` gana `[access]` y `[leases]` (5m, máx 1h).
- Leases: si no se hace commit, se revierte al vencer, al parar el daemon (SIGTERM/SIGINT/SIGHUP,
  con `sigwait`) o, tras una caída, al arrancar (journal de `hpcd-fix`). El hilo de vencimientos
  duerme hasta el próximo, sin sondeo. Dos leases no pueden tocar el mismo recurso. Un `--pid`
  no se puede commitear.
- `sd_notify` READY=1 para la unidad `Type=notify`, sin dependencias.
- `hpcctl`: `fix --rule <id> [--lease 10m]`, `status`, `commit <n>`, `revert <n>`. Sin `--rule`
  ni `--dry-run`, `fix` explica qué usar. `parse_duration` pasa a `hpcd-core`.
- Commit en v0.3: el cambio queda hasta reiniciar; perfiles que sobrevivan reinicios son de v0.4.
- Probado en WSL con el daemon como usuario normal: aplicar `thread-migration` a `jitter-demo
  --noise` con lease de 6s, ver el estado, commit rechazado por ser `--pid`, vencimiento que
  revierte, y SIGTERM que revierte un lease de 1h.

### 2026-10-23 · v0.3 parte 2: `hpcctl fix --dry-run`

- `hpcd-core::Plan`: regla, resumen, cambios, riesgo, reboot, conflictos activos y nota.
- Cada acción del registro tiene ahora su planificador (`hpcd-rules/src/plan.rs`), que convierte
  la observación en cambios concretos: IRQs a mover, cores a cambiar de governor, hilos a fijar.
- `housekeeping_cores`: cores sin hilos calientes ni hermanos SMT de uno, no aislados, en el
  mismo nodo NUMA si se puede. En la captura: cores 2,6.
- Conflictos activos: los de `[fix.conflicts]` cuya regla de conflicto (`irqbalance-active`,
  `tuned-active`) coincidió en el audit.
- `move_tasks_off_cpu` todavía no lista las tareas: la captura no guarda las de todo el sistema;
  se leerán al aplicar.
- `hpcctl fix --dry-run` con el formato de la pantalla 03 (`[1/5] regla → resumen`), los
  cambios debajo, conflictos con ⚠ y las propuestas de boot aparte. Sin `--dry-run` dice que
  falta el daemon. Orden por `order` de cada regla, como el diseño.
- Encontrado al probar en vivo con `jitter-demo --noise`: los hilos calientes ocupan los 8 cores
  y la propuesta era `isolcpus=…,0-7`. Ahora no propone aislar si no quedan cores para el
  sistema, y lo explica.

### 2026-10-23 · v0.3 parte 1: `hpcd-fix`

- Plan de v0.3 en 6 partes: (1) aplicar y revertir cambios, (2) `fix --dry-run`, (3) daemon con
  socket, `SO_PEERCRED`, `policy.toml` y leases, (4) `fix --interactive` con medición antes y
  después y guardrails, (5) herramientas MCP de acción, (6) `hpcctl run`.
- `CpuSet` pasa a `hpcd-core` (serializable); `hpcd-probe` lo reexporta igual.
- `hpcd-core::Change`: `IrqAffinity`, `CpuGovernor`, `ThreadAffinity`, `CpuDmaLatency`.
  `move_tasks_off_cpu` será varios `ThreadAffinity`; `propose_kernel_cmdline` no produce cambios.
- Nuevo crate `hpcd-fix`. Por cada cambio: leer el valor actual, guardarlo en el journal (un
  JSON por grupo, escrito con rename), escribir y volver a leer. Si el kernel deja otro valor o
  algo falla, revierte lo ya aplicado. Al arrancar, revierte lo que dejó una caída.
- `CpuDmaLatency` se mantiene mientras el archivo siga abierto (`Hold`); revertir es cerrarlo.
  Tras una caída el kernel ya lo soltó.
- `commit` olvida el snapshot y conserva el cambio. Persistir entre reinicios es de v0.4.
- Hilo o IRQ que ya no existe al revertir: se da por revertido.
- El acceso al sistema pasa por el trait `System`: tests con un sistema falso (en Windows también)
  y un test en Linux que fija la afinidad de un `sleep` real y la restaura. `cpu_dma_latency`
  probado en WSL (ahí es escribible sin root).
- v0.2 marcada como hecha en el roadmap del README.

### 2026-10-23 · `hpcctl mcp`: herramientas de solo lectura

- Servidor MCP por stdio escrito a mano (JSON-RPC por líneas): `initialize` (negocia
  `2025-06-18`, `2025-03-26`, `2024-11-05`), `ping`, `tools/list`, `tools/call`. Solo agrega
  `serde_json`, que ya estaba en el lockfile. Se descartó el SDK oficial (`rmcp`) porque trae
  tokio y muchas dependencias. `cargo audit`: 0 avisos en 75 dependencias.
- Herramientas del README: `system_topology`, `process_profile`, `numa_placement`, `irq_map`,
  `cpu_power_state`, `audit`, `explain_rule`, `measure`. Todas con `readOnlyHint`. Devuelven el
  mismo texto que la CLI, sin colores.
- `hpcd-probe`: `observe_host` (observación sin target), cambios de contexto por hilo,
  frecuencias (`scaling_cur/min/max_freq`) y cachés (`cpu0/cache/index*`).
- `audit` y `measure` de `main.rs` ahora devuelven texto (`audit_report`, `measure_report`); la
  CLI lo imprime y sale con el código, MCP lo envía.
- `hpcctl mcp` se niega a correr como root (README: "AI agents never get privileges"). Por eso
  `measure` devuelve un error claro hasta que exista el daemon (v0.3). En MCP, `measure` usa
  5 × 4s por defecto para no pasar los timeouts de los clientes.
- `hpcctl mcp --snapshot <dir>`: las herramientas leen una captura. Sirve para probar en Windows.
- Probado en WSL como usuario: topología con cachés, power, process_profile; `measure` da el
  error de privilegios; como root se niega a arrancar.
- Conectar: `claude mcp add hpcd -- hpcctl mcp`.

### 2026-10-23 · Costo del programa eBPF

- Medido por programa en WSL: `sched_wakeup` ~900ns, `sched_switch` ~720ns–1.2µs por evento.
  Con `sched_wakeup` vacío (solo `return 0`) cuesta ~500ns: casi todo el costo en WSL es fijo
  (la VM y las propias estadísticas de eBPF), y entre corridas varía ±300ns.
- Cambio: en el namespace de PID raíz (máquina normal, sin contenedor) `hpcctl` escribe los hilos
  del target directo en `GLOBAL` y `learn_global` sale enseguida: sin
  `bpf_get_ns_current_pid_tgid` ni búsquedas en `TARGETS`/`GLOBAL` en cada `sched_switch`.
- En contenedores y WSL se sigue traduciendo. Un caché negativo ahorraría ~50ns a cambio de
  bastante complejidad (invalidarlo al agregar hilos); no se hizo.
- El camino del namespace raíz no se puede probar en WSL; queda para la máquina física.

### 2026-10-23 · Detección de SMI (`smi-activity`)

- `SysRoot::read_msr(cpu, registro)`: en vivo lee `/dev/cpu/N/msr` (8 bytes en el offset del
  registro, necesita root y el módulo `msr`). En capturas se guarda como texto en
  `/dev/cpu/N/msr-0x34`, así `audit --snapshot` también lo ve.
- `observe` lee `MSR_SMI_COUNT` (0x34, Intel) del cpu 0 en las dos muestras; el contador es de
  32 bits y se maneja la vuelta.
- Check `smi_count` (`min_per_minute`) y regla `rules/firmware/smi-activity.toml` (HIGH, umbral
  6 por minuto) con el mensaje de la pantalla 12 y propuesta para Dell y genérica.
- Si no se puede leer el contador, `audit --firmware` lo dice:
  `smi-activity not checked: needs root and the msr module (Intel only)`.
- La captura `order-gateway` tiene 9 SMIs en 10s; `capture` ahora lista también `/dev`.
- Diferencia con la pantalla 12: falta "31 coincide with p99 spikes > 300µs". Necesita medir
  latencia durante el audit; queda pendiente.
- WSL no tiene el módulo `msr`: probado solo con la captura.
- `--sample 60` da una tasa más fiable que los 3s por defecto.

### 2026-10-23 · Overhead propio

- Durante la medición se activan las estadísticas de eBPF del kernel (`BPF_ENABLE_STATS`, kernel
  5.8+, con `aya::sys::enable_stats`; se desactivan al soltar el fd) y se suma `run_time` y
  `run_count` de los tres tracepoints.
- La CPU de `hpcctl` sale de `/proc/self/stat` y su memoria de `VmRSS`.
- `measure` lo muestra como la pantalla 06:
  `hpcd itself 1.3% cpu during measurement · eBPF 1.3µs per event · 6 MiB rss`. Si el kernel no
  permite las estadísticas, dice `(eBPF not counted)`.
- En WSL salen 1.3µs por evento, bastante. Parte es la VM (leer el reloj es caro) y parte las
  propias estadísticas. Mejora posible: `learn_global` llama a `bpf_get_ns_current_pid_tgid` en
  cada `sched_switch` de hilos que no son del target; se puede recordar los que ya se revisaron.

### 2026-10-23 · Métricas de guardrails

- La lectura de carga pasa a `hpcd-probe/src/load.rs` y ahora también lee page faults
  (`minflt` + `majflt` de `/proc/<pid>/stat`), RSS (`VmRSS`), energía de paquete RAPL
  (`/sys/class/powercap/intel-rapl:N/energy_uj`, con la vuelta del contador) y NUMA misses
  (`numastat` de cada nodo, de todo el sistema y solo con más de un nodo).
- `hpcd-measure::guardrails` promedia sobre los bloques medidos. `measure` lo muestra bajo el
  resumen como la pantalla 03: `cpu 61% · power 142W · rss 3.1 GiB · faults 120/s`. Lo que la
  máquina no expone no aparece.
- Throughput sin `--metric` no se puede medir; queda para cuando exista `--metric`.
- En v0.2 solo se registran. Los límites de `policy.toml` se aplican en v0.3, cuando `fix`
  compare antes/después.
- WSL no tiene RAPL ni NUMA: la línea muestra cpu, rss y faults. El test de RAPL solo corre en
  Linux (Windows no permite `:` en nombres de archivo).

### 2026-10-23 · Chequeo de estabilidad (pantalla 04)

- `measure` lee cada segundo la carga del target: CPU (`/proc/<pid>/stat`), cambios de contexto
  (`voluntary` + `nonvoluntary` de cada hilo) y wakeups (el histograma eBPF).
- Agrupa por ventana y compara: inestable si la CPU varía más de 10 puntos (o 20% de la media,
  para procesos con muchos cores) o si wakeups/s o switches/s cambian más de 25% y más de 50/s.
- Si es inestable, muestra las ventanas como la pantalla 04 (p99 en amarillo con +50%, rojo con
  +100% respecto a la primera), no da veredicto y sale con código **2** (no 1, para que CI no lo
  confunda con una regresión).
- Si es estable, el resumen dice `load stable ✔` (como el README).
- `--wait-stable 10m`: sigue midiendo y se queda con las últimas N ventanas en cuanto son
  estables; si se acaba el tiempo, sale con código 2 y dice cuánto esperó.
- Solo se sugiere `--wait-stable`: `--metric` y `--bench` no existen todavía.
- La pantalla 04 es de `fix --auto` y usa "requests"; `measure` no conoce las peticiones de la
  app y usa wakeups y switches en su lugar.
- Probado en WSL: `jitter-demo --noise` estable (exit 1 por el `--assert`); un script de Python
  que agrega hilos cada 5s da inestable (exit 2); con `--wait-stable 30s` espera 7s y mide.
- Arreglado: las filas de `measure` sin `--assert` terminaban con espacios.

### 2026-10-23 · `measure` funciona en WSL

- `bpf-linker`: binario oficial de las releases de `aya-rs/bpf-linker`. `cargo install bpf-linker`
  falla (pide LLVM 23 del sistema; Ubuntu 24.04 trae 20) y su README lo desaconseja.
- Primer build: compiló, el verificador aceptó el programa y los tracepoints se conectaron,
  pero 0 wakeups del target.
- Causa: WSL2 corre cada distro en su propio PID namespace (inodo `4026532874`; el raíz es
  `4026531836`). Los tracepoints reportan pids globales y `hpcctl` conoce los del namespace.
  Lo mismo pasa en Docker y Kubernetes.
- Arreglo: en `sched_switch`, el programa pregunta el pid del hilo que sale con
  `bpf_get_ns_current_pid_tgid` y, si es del target, aprende su pid global (mapa `GLOBAL`).
- Contadores de diagnóstico en el programa (wakeups, switches, eventos del target, registrados):
  si no hay datos, `measure` dice en qué paso se perdieron.
- Resultado sobre `jitter-demo --noise` (3 × 5s): p99 56µs [34–152], p99.9 1.47ms [1.03–2.00];
  `--assert "p99.9 < 1ms"` falla con código 1. Sin root: error claro pidiendo CAP_BPF/CAP_PERFMON.

### 2026-10-22 · v0.2 empieza: `hpcctl measure` con eBPF (aya)

- Librería eBPF: **aya** (todo Rust). Versiones fijadas: `aya =0.14.0`, `aya-ebpf =0.2.1`,
  `aya-build =0.2.1`. Hay forks con nombres parecidos (`aya-friday`, `profile-bee-aya`, …): solo
  usar los de `aya-rs/aya`. `cargo audit`: 0 avisos en 75 dependencias.
- `hpcd-ebpf`: programa en los tracepoints `sched_wakeup`, `sched_wakeup_new` y `sched_switch`.
  Mide el tiempo desde que un hilo del target queda ejecutable hasta que corre, en un histograma
  por CPU de 592 buckets (±3%). Los offsets de los campos se leen del `format` del kernel.
- `hpcd-measure`: carga el programa, añade los hilos del target cada segundo, guarda un
  histograma por segundo y calcula p50/p99/p99.9 con intervalo de confianza del 95% por
  bootstrap sobre esos bloques.
- `hpcctl measure --unit X [--windows 5 --window 20] [--assert "p99 < 200µs"]`, formato de la
  pantalla 13; sale con código 1 si falla un `--assert`. El veredicto usa la estimación puntual.
- Compilar en Linux necesita nightly + `bpf-linker`; `AYA_BUILD_SKIP=1` compila sin `measure`.
  En Windows compila y pasa los tests, pero `measure` dice "needs Linux".
- `rust-version` sube a 1.98 (lo pide `aya-build`). Se quita `lto` del perfil release.

### 2026-10-22 · Banner (pantalla 14)

- Pantalla 14 en el diseño: "HPCD" en pixel art con degradado violeta → cian y tips.
- `hpcctl` sin argumentos lo muestra si la salida es una terminal; si no, imprime la ayuda de
  clap, así no ensucia scripts.
- Medios bloques (`▀▄█`): 7 filas de píxeles en 4 líneas, píxeles cuadrados, color por columna.
- Sin la sombra del diseño: en la terminal se desplaza un píxel entero (no un tercio), tapa los
  espacios entre letras y la palabra deja de leerse.

### 2026-10-22 · v0.1 probada en WSL

- `audit`, `audit --firmware`, `explain`, `capture --show` y el audit de la captura de prueba
  funcionan en WSL. Sin DMI (VM), el encabezado de firmware muestra solo la CPU.
- El mensaje de `irq-on-hot-core` se partía en terminales normales. Ahora agrupa las colas de un
  dispositivo como el diseño (`eth0 (rx-0..3)`) y pone la tasa en la segunda línea.
- v0.1 marcada como hecha en el roadmap del README.

### 2026-10-22 · Cierre de v0.1: `sub-numa-clustering` y propuesta de firmware

- `hpcd-probe` lee la máquina sin root: fabricante, modelo y BIOS (`/sys/class/dmi/id`), modelo
  de CPU (`/proc/cpuinfo`) y el socket de cada CPU (`physical_package_id`).
- Check `sub_numa_clustering`: más nodos NUMA que sockets. Regla `sub-numa-clustering` (firmware,
  INFO).
- Las reglas de firmware llevan `[propose]` con pasos por fabricante (`dell`, `hpe`, `lenovo`,
  `supermicro`) y un `generic` obligatorio, validado al cargar.
- `audit --firmware` muestra la máquina en el encabezado y, al final, la propuesta para su
  fabricante, como la pantalla 12. `explain` muestra la sección PROPOSE.
- Solo hay rutas de menú de Dell (y de Supermicro para SMT). Las demás marcas usan el paso
  genérico hasta que alguien con esos equipos las aporte.

### 2026-10-22 · `thread-migration`, `smt-sibling-busy` y `remote-numa-memory`

- `hpcd-probe` lee las migraciones de cada hilo (`/proc/<pid>/task/<tid>/sched`), sus CPUs
  permitidas (`status`), la ocupación de cada CPU (`/proc/stat` en las dos muestras), los
  nodos NUMA y la memoria del proceso por nodo (`numa_maps`).
- Privacidad: `numa_maps` lleva rutas de archivos mapeados; la captura guarda una versión
  depurada con solo los conteos por nodo.
- Checks `thread_migration`, `smt_sibling_busy`, `numa_remote_memory`. Acciones registradas
  `pin_thread` y `move_tasks_off_cpu`. `remote-numa-memory` es solo reporte.
- El audit de `order-gateway` muestra las 8 filas de la pantalla 01, con "5 fixable at runtime"
  como en el diseño.

### 2026-10-22 · `isolcpus-missing` y `smt-enabled`

- `hpcd-probe` lee `/sys/devices/system/cpu/isolated`, `smt/active` y los hilos hermanos de
  cada core (`topology/thread_siblings_list`). `CpuPower` pasa a llamarse `CpuInfo`.
- Check `cpu_isolation`: algún core caliente no está aislado. Regla `isolcpus-missing` (boot,
  INFO): propone `isolcpus`, `nohz_full` y `rcu_nocbs` con la acción `propose_kernel_cmdline`,
  que nunca se aplica.
- Check `smt_siblings`: SMT activo y un core caliente comparte núcleo físico. Regla `smt-enabled`
  (firmware, LOW), solo con `audit --firmware`.
- La captura `order-gateway` tiene SMT (cpu 3 y 7 hermanas) y ningún core aislado: el audit
  muestra 5 filas de la pantalla 01, y `--firmware` la fila `smt-enabled` de la pantalla 12.

### 2026-10-22 · `hpcctl capture`

- `capture` ejecuta la misma observación que `audit`, pero registra cada archivo que lee, y
  guarda exactamente esos archivos en `t0/` y `t1/` más `capture.toml`. Lo que `audit` necesita
  siempre está en la captura.
- `--show` lista cada archivo y su tamaño sin escribir nada.
- No sobrescribe: falla si el directorio de salida existe y no está vacío.
- Privacidad: con `--exe` ya no se lee el `cmdline` de todos los procesos (puede contener
  contraseñas); solo se guarda la ruta del ejecutable de los procesos que coinciden.
- Test de ida y vuelta: capturar la captura de prueba, escribirla, leerla y obtener lo mismo.
- Formato: directorio. El archivo comprimido (`.tar.zst`) queda para más adelante.
- Probado de punta a punta: captura de `systemd-journald.service` en WSL (40 archivos, ~7 KiB)
  y audit de esa captura desde Windows con el mismo resultado que en vivo.
- Columnas de la salida de `capture` alineadas.

### 2026-10-22 · Reglas de conflicto: `irqbalance-active` y `tuned-active`

- `hpcd-probe` lee la lista de procesos (`/proc/*/comm`) y el perfil activo de tuned
  (`/etc/tuned/active_profile`).
- Check `process_running`: algún proceso con uno de los nombres dados está corriendo.
- Check `tuned_profile`: tuned corre y tiene un perfil activo.
- Las dos reglas son de solo detección (WARN, sin fix). Más adelante, en v0.3, se revisarán
  antes de cada fix.
- La captura `order-gateway` tiene ahora `irqbalance` corriendo (pid 812): el audit muestra 4
  de las 8 filas de la pantalla 01.

### 2026-10-22 · Reglas de energía: `deep-cstates` y `powersave-governor`

- `hpcd-probe` lee `/sys/devices/system/cpu/cpu*/cpuidle/state*` y `cpufreq/scaling_governor`.
  Si no existen (VMs, WSL) no hay datos y las reglas no se disparan.
- Check `cpuidle_deep_states`: C-states habilitados con latencia de salida mayor que
  `max_exit_latency_us` (20 µs) en los cores de hilos calientes.
- Check `cpufreq_governor`: governor de los cores calientes dentro de una lista
  (`powersave`, `conservative`).
- Acciones registradas (todavía sin implementar): `hold_cpu_dma_latency`,
  `set_cpufreq_governor`, cada una con su reversión.
- La captura de prueba pasa a llamarse `rules/tests/order-gateway/` y ahora dispara las tres
  reglas, como la pantalla 01 del diseño.

### 2026-10-22 · Revisión de seguridad de dependencias

- Motivo: ataque a crates.io en agosto de 2026 (`arrayref`, `internment`, `append-only-vec`
  con el dropper `proc-macro1`) y el compromiso de la CA eMudhra en octubre de 2026, que
  afecta a `webpki-roots` / `rustls-webpki`.
- `cargo audit` (1349 avisos de RustSec): 0 vulnerabilidades en las 42 dependencias.
- El proyecto no usa ninguna librería de TLS/SSL (`rustls`, `openssl`, `ring`, `webpki`) ni
  ninguno de los crates comprometidos.
- Cachés de cargo: en Windows solo está `arrayref 0.3.9` (de otro proyecto), que es la última
  versión limpia. En WSL no hay ninguno. `proc-macro1` no se descargó nunca.
- Repetir `cargo audit` antes de cada versión y al agregar dependencias.

### 2026-10-22 · Primera prueba en vivo (WSL Ubuntu 24.04)

- Rust instalado en WSL; `hpcctl` compila e instala en Linux sin cambios.
- `audit --pid` lee procesos e hilos reales sin errores. 0 hallazgos, como se esperaba en una
  VM de Hyper-V.
- Corregido: el encabezado con `--pid` no tenía separador (`pid 1148 1 threads`), plural de
  "thread", y la salida con 0 hallazgos ahora dice `✔ no issues found`.
- Ojo al probar: `cargo run … &` deja en `$!` el pid de cargo, no el del programa. Lanzar el
  binario directamente.
- `audit --pid` sobre `jitter-demo --noise`: 9 hilos detectados.
- `audit --unit systemd-journald.service` encuentra la unidad con cgroups v1 (WSL).

### 2026-10-22 · `hpcctl` como CLI independiente

- Las reglas de `rules/` se meten en el binario al compilar (`crates/hpcd-rules/build.rs`).
  `hpcctl` funciona desde cualquier carpeta sin configurar nada.
- `--rules <dir>` o `HPCD_RULES` cargan reglas de una carpeta, para probar reglas nuevas
  sin recompilar.
- Test que verifica que las reglas del binario son las mismas que las de `rules/`.

### 2026-10-22 · Primer check real: `audit` funciona

- `hpcd-probe` lee `/proc/interrupts`, `/proc/<pid>/task/*/stat`, `/proc/irq/*/smp_affinity_list`
  y los `cgroup.procs` de una unidad systemd o de un cgroup.
- Toma dos muestras separadas por un intervalo (`--sample`, 3 s por defecto) y calcula IRQs/s
  por core y uso de CPU por hilo.
- Hilos "calientes": los que usan al menos 5% de un core.
- Check `irq_affinity_overlaps`: suma las IRQs de dispositivo (no `LOC`, `RES`…) que caen en el
  core de un hilo caliente; si pasan `min_irq_rate`, hay hallazgo.
- Formato de captura: `capture.toml` + `t0/` + `t1/` (ver `docs/architecture.md`).
- Fixture `rules/tests/order-gateway/`: 8 cores, eth0 rx-0..3 en el core 3 (8,300 IRQs/s),
  hilo `gw-io` al 95%.

### 2026-10-22 · Salida alineada con el diseño

- Paleta y badges sacados del `.pen` (`crates/hpcctl/src/style.rs`).
- `explain` con secciones WHY / EVIDENCE / FIX / CONFLICTS / REFERENCES.
- `audit` con encabezado, filas alineadas y pie `N fixable at runtime`.
- Tests que comparan la salida con las pantallas 01 y 02.
- Nueva categoría `firmware` y opción `audit --firmware`.
- Cada acción del registro declara cómo se revierte.

### 2026-10-22 · Estructura del proyecto

- Workspace Cargo según el "Project layout" del README.
- `rules/_template.toml` y la primera regla `irq-on-hot-core`.
- `packaging/` con la unidad systemd y la política por defecto.
- Licencias MIT y Apache-2.0.

## Decisiones

- **El diseño `.pen` es la referencia de la salida.** Cada comando nuevo se compara con su
  pantalla antes de darlo por hecho.
- **El texto normal usa el color por defecto de la terminal**, no el `#D6DCE4` del diseño,
  para que se lea también con fondo claro.
- **Todo se lee a través de `SysRoot`**, para que el sistema real, las capturas y los tests usen
  el mismo código.
- **La reversión de cada acción la define el motor**, no la regla.
- **Pocos comentarios en el código.** Se mantienen los `///` de clap (son el `--help`) y los del
  template de reglas.

## Pendiente / próximos pasos

1. v0.3 parte 4: `fix --interactive` (medición antes/después y guardrails).
2. Probar el daemon como servicio systemd real (`Type=notify`, grupo `hpcd`).
3. Probar en una máquina física cuando haya una a mano.
4. `capture --show`: ordenar rutas numéricamente (`/proc/8` antes que `/proc/24`).
5. `explain --unit` con la evidencia en vivo, como en la pantalla 02.
6. Validar los parámetros de cada check al cargar las reglas, no solo al evaluarlas.
7. Medir el costo por evento de eBPF en una máquina física (namespace raíz).
8. `smi-activity`: correlacionar los SMIs con picos de p99, como en la pantalla 12.
9. Probar `smi-activity` en una máquina física Intel.
