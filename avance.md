# Avance

Registro de lo que se va haciendo en hpcd. Lo más reciente arriba.

## Estado actual

**Versión en curso:** v0.1 — Audit

| Pieza | Estado |
|---|---|
| Workspace y crates (`hpcd-core`, `hpcd-probe`, `hpcd-rules`, `hpcctl`, `jitter-demo`) | ✅ |
| Formato de reglas, carga y validación | ✅ |
| Estilo de terminal según `design/hpcctl-terminal.pen` | ✅ |
| `hpcctl explain` (pantalla 02) | ✅ sin evidencia en vivo |
| `hpcctl audit` (pantalla 01) | ✅ con 1 regla |
| `hpcctl audit --snapshot` | ✅ |
| `hpcctl audit --firmware` (pantalla 12) | 🟡 opción lista, sin reglas de firmware |
| `hpcctl capture` (directorio, `--show`) | ✅ |
| CLI instalable (`cargo install`), reglas dentro del binario | ✅ |
| Reglas: `irq-on-hot-core`, `deep-cstates`, `powersave-governor` | ✅ con fixture |
| Reglas restantes del set inicial (`thread-migration`, `smt-sibling-busy`, `remote-numa-memory`, `isolcpus-missing`) | ❌ |
| Reglas de conflictos: `irqbalance-active`, `tuned-active` | ✅ |
| Reglas de conflictos restantes (`systemd-cpuaffinity`, `cpuset-restricted`, `k8s-cpu-manager`) | ❌ |

## Cómo probar

```powershell
cargo test --workspace
cargo run -p hpcctl -- explain irq-on-hot-core
cargo run -p hpcctl -- audit --unit order-gateway.service --snapshot rules/tests/order-gateway
cargo run --release -p jitter-demo -- --noise
```

`audit` en vivo solo funciona en Linux. En Windows usar `--snapshot`.

Como CLI instalada:

```powershell
cargo install --path crates/hpcctl
hpcctl audit --unit order-gateway.service --snapshot rules/tests/order-gateway
```

## Registro

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

1. Resto de reglas del set inicial: `thread-migration`, `smt-sibling-busy`,
   `remote-numa-memory`, `isolcpus-missing`.
2. Reglas de firmware que no necesitan root (`smt-enabled`, `sub-numa-clustering`).
3. `explain --unit` con la evidencia en vivo, como en la pantalla 02.
4. Validar los parámetros de cada check al cargar las reglas, no solo al evaluarlas.
