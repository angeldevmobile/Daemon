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
| `hpcctl capture` | ❌ |
| CLI instalable (`cargo install`), reglas dentro del binario | ✅ |
| Reglas: `irq-on-hot-core` | ✅ con fixture |
| Reglas restantes del set inicial | ❌ |
| Reglas de conflictos (`irqbalance-active`, …) | ❌ |

## Cómo probar

```powershell
cargo test --workspace
cargo run -p hpcctl -- explain irq-on-hot-core
cargo run -p hpcctl -- audit --unit order-gateway.service --snapshot rules/tests/irq-on-hot-core
cargo run --release -p jitter-demo -- --noise
```

`audit` en vivo solo funciona en Linux. En Windows usar `--snapshot`.

Como CLI instalada:

```powershell
cargo install --path crates/hpcctl
hpcctl audit --unit order-gateway.service --snapshot rules/tests/irq-on-hot-core
```

## Registro

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
- Fixture `rules/tests/irq-on-hot-core/`: 8 cores, eth0 rx-0..3 en el core 3 (8,300 IRQs/s),
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

1. `hpcctl capture` para generar capturas reales (`t0/`, `t1/`) desde Linux.
2. Probar `audit` en vivo en WSL (Ubuntu no tiene Rust instalado todavía: `rustup`).
3. Reglas `deep-cstates` y `powersave-governor` (nuevos checks de cpufreq/cpuidle).
4. Regla de conflicto `irqbalance-active`.
5. `explain --unit` con la evidencia en vivo, como en la pantalla 02.
6. Validar los parámetros de cada check al cargar las reglas, no solo al evaluarlas.
