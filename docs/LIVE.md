# synth-core como sinte portable con superficie de livecoding

> Análisis independiente, 2026-09-15. Todo lo que afirma está medido en este
> repo; los números salen de `core/tests/live_probe.rs` (sonda temporal, en
> release) y de la CLI. Donde no medí, lo digo.

## Resumen en una pantalla

1. **La partición voces/performance es correcta, pero no es un lenguaje nuevo.**
   El `.synth` ya tiene las dos mitades: `module` + `bus` + `master` + sends
   globales son el *rig* (se tunea offline); `pattern` + `track` son *lo que
   suena ahora*. `scene` + `arrange` son la capa de composición, y el motor ya
   corre sin ellas: un archivo de 12 líneas sin escenas compila, renderiza los
   compases completos y loopea para siempre. La superficie de livecoding existe
   hoy como subconjunto. Lo que falta es un `use "rig.synth"` para no repetir el
   rig, y una forma de tocar sin recompilar dos veces la misma cosa.

2. **Las cadenas de efectos viven en los dos lados, y el criterio es la vida del
   estado.** Un insert de track (`out > saturate > master`) es parte de la voz:
   no tiene cola, se reconstruye con ella. Sends, retornos, buses y master son
   parte de la sesión: tienen segundos de cola (reverb 15 s, delay con 0.74 de
   feedback) y esa cola tiene que sobrevivir a cualquier edición. Hoy no
   sobrevive a ninguna edición estructural.

3. **El motor aguanta lo que hace falta en vivo.** Compilar el tema más grande
   del corpus lleva menos de 1 ms. El peor bloque de audio usa el 7 % del
   presupuesto. El camino de audio no reserva memoria. No hay nada estructural
   que impida tocar.

4. **El hot-swap no está en condiciones.** Encontré cuatro defectos, tres de
   ellos medibles en muestras. El más grave está en el motor, no en el swap:
   `current_bar` cambiaba un 16avo antes del compás real, y eso ya afectaba a
   los 25 temas renderizados sin que nadie lo notara.

5. **El primer paso es `synth play` y `synth watch`, pero no como lo pide el
   roadmap.** La lógica de swap vive en `wasm/src/lib.rs`, no en el core. Si la
   CLI la copia, hay dos implementaciones de la parte más delicada del producto,
   que es exactamente lo que "Qué no hacer" prohíbe. El primer paso real es una
   `LiveSession` en el core que el WASM y la CLI compartan, arreglada, y recién
   encima el `play`.

## Lo que medí

### El motor tiene margen de sobra

| tema | parse | compile | build engine | render | peor bloque | presupuesto por bloque |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `psytech_goa` (743 líneas) | 722 µs | 87 µs | 43 µs | 40× tiempo real | 206 µs | 2902 µs |
| `hitech_psy` (1075 líneas) | 319 µs | 43 µs | 39 µs | 25× | 180 µs | 2902 µs |
| `dub_techno` | 159 µs | 21 µs | 30 µs | 48× | 109 µs | 2902 µs |

Compilar es despreciable: se puede recompilar en cada tecla. El peor bloque
deja 14× de margen, así que una Raspberry Pi 4 (unas 8 a 10 veces más lenta
que esta máquina en punto flotante) también entra, con menos holgura. Esto no
lo medí en la Pi; es una extrapolación.

Esto cambia el diseño: **no hace falta un camino rápido para ser rápido**. El
diff existe para no reconstruir el motor, pero reconstruirlo cuesta 40 µs. Lo
que sí cuesta es perder el estado (colas, envolventes, fase de los LFO). El
problema es de continuidad, no de velocidad.

### El contador de compás iba un 16avo adelantado

`current_bar` se incrementaba al final de `advance_step`, después de disparar
el último step del compás. Así que durante todo el step 15 el motor ya decía
"compás siguiente". A 120 BPM:

```
current_bar -> 1 en la muestra 82687   (el compás 1 empieza en la 88200)
current_bar -> 2 en la muestra 170887  (esperado 176400)
```

Tres consecuencias, todas silenciosas:

- **Cada cambio de escena mataba la nota del step 15.** `apply_scene` corre
  al cruzar el contador, o sea junto con el disparo del último step: dispara la
  nota y la suelta en el mismo sample. Medido con un `keys` tocando en todos
  los steps con `gate 0.9`: step 13 audible el 100 % del step, step 14 el
  100 %, **step 15 el 0 %**. Pasa en cada transición de los 25 temas. Después
  del fix, 100 % en los tres.
- **Todo render con arreglo perdía el último 16avo.** `running = false` se
  activaba un step antes del final. `hitech_psy`: 4 775 680 muestras escritas
  contra 4 779 871 esperadas. Tempo, freeze, `reverb_mix` y las automatizaciones
  también arrancaban un step antes.
- **El hot-swap del navegador entraba un 16avo antes del downbeat**, porque
  mira `current_bar`. Cada edición estructural corría toda la canción 125 ms
  hacia adelante.

El fix está en el árbol de trabajo (`advance_step` cruza la línea de compás
antes de disparar, comparando `global_step / steps_per_bar` contra
`current_bar` para que `start_from_bar` no cuente doble). La suite completa
pasa con el fix: 214 tests más los 6 de la sonda, todos en verde. Los WAV de
referencia en `core/test_output/probes/` cambian porque ahora tienen el
último 16avo. Ningún test existente detectaba el problema, porque todos
verifican que algo suene y no cuándo.

### El swap corta el bloque en el lugar equivocado

Con el contador arreglado, reproduje la lógica de `wasm/src/lib.rs` en nativo
y miré el compás del swap con un bombo en el step 0:

```
onsets del bombo (muestras desde el compás): referencia [14]   swapeado [14, 377]
```

Dos bombos con 8 ms de diferencia. El WASM detecta el cambio de compás
*después* de renderizar el bloque, así que el motor viejo ya disparó el
downbeat, y el nuevo lo vuelve a disparar al principio del bloque siguiente.
Un flam en cada edición. La solución es partir el bloque en la muestra exacta:
el motor sabe cuántas muestras faltan hasta el compás (`sample_counter` y
`current_step_duration` lo dicen), así que se renderiza el viejo hasta ahí y
el nuevo desde ahí.

### El swap tira la cola

Pad con acorde tenido, `reverb size=1.0`, `reverb_send 0.7`, `delay_send 0.5`.
Swap estructural a un tema idéntico (un patrón sin usar agregado). Referencia
contra swapeado, RMS por ventana de 100 ms después del compás:

```
t+0ms    -19.5 dB   -20.8 dB   (-1.3)
t+100ms  -21.5 dB   -27.1 dB   (-5.6)
t+200ms  -21.9 dB   -25.6 dB   (-3.7)
```

El motor nuevo nace con la reverb vacía, el delay vacío, el `capture` vacío y
el acorde se redispara desde el ataque. El crossfade de 12 ms tapa el click,
no la cola. Para dub o ambient, donde "la señal seca es casi incidental"
(ítem 11.9 del roadmap), cada edición es un corte.

### El camino rápido saltea el compilador

En `load_source`, si el diff no es estructural, **el AST nuevo nunca se
compila**. La validación de rango vive en `compile_module_def`, así que en el
navegador `cutoff 7.0` se aplica clampeado en silencio mientras `synth check`
lo rechaza. Es la misma clase de bug que la lista de no-ops silenciosos: el
mismo texto significa dos cosas según el camino que tome. Compilar cuesta
87 µs; no hay razón para no hacerlo siempre.

### La reserva de memoria del swap está en el hilo de audio

`from_compiled` reserva (instrumentos, cadenas, buffers de capture). En el
WASM eso corre adentro del worklet, en el mismo hilo que `process`. Hoy no se
nota porque compilar es rápido, pero es la regla que 8.10 estableció y que la
CLI nativa no puede violar: en nativo, el motor se construye en el hilo que
mira el archivo y se pasa al hilo de audio ya armado; el motor retirado vuelve
por otro canal para que su `drop` no ocurra en el callback.

## Las respuestas

### ¿Es correcta la partición?

Sí, pero la unidad no es "archivo de config vs archivo de performance", es
**qué cambia a qué velocidad**:

| capa | contiene | cambia | estado que carga |
| --- | --- | --- | --- |
| rig | `module`, `instrument`, `bus` + cadena, `master`, `delay`/`reverb` globales, retornos | offline, con medición | colas largas (segundos), buffers de capture |
| performance | `pattern`, `track` (play, using, level, pan, sends, inserts) | en vivo, por compás | notas tenidas, fase del arp, filtros de insert |
| composición | `scene`, `arrange`, `auto` | nunca en vivo | automatizaciones |

Las tres ya existen en el DSL con esos nombres. La partición está hecha; lo
que falta es que el hot-swap la respete: **lo que no cambió en el texto
conserva su estado en el motor**. Con esa regla, editar un patrón no toca la
reverb, cambiar un `level` no toca nada más, y agregar un track no redispara
el pad que ya sonaba.

Dos cosas prácticas que faltan:

- `use "rig.synth"` en el archivo de performance. El core es `no_std` y no
  tiene filesystem, así que lo resuelve la CLI (concatena antes de parsear) y
  el core recibe un solo fuente. No hace falta tocar el parser.
- Sin escenas ni `arrange` el motor loopea los tracks de arriba para siempre.
  Ese es el modo live y ya anda. Hay que documentarlo como tal y que
  `single_scene` y `sidechain_without_kick` no lo traten como un tema a medio
  escribir.

### ¿Dónde van las cadenas de efectos?

En los dos lados, y ya están en el lado correcto:

- **Inserts de track** (`out > highpass > phaser > master`): parte de la voz.
  Estado corto (un filtro, un phaser). Se reconstruyen con el track y no se
  escucha, siempre que el swap sea a la muestra exacta.
- **Buses, sends globales, retornos, master**: parte de la sesión. Estado
  largo. Tienen que sobrevivir a todo lo que no los toque. Hoy no sobreviven a
  nada.

La pregunta abierta del roadmap (11.18, "cadenas por escena precompiladas e
intercambiadas con un swap") es la misma pregunta: un cambio de cadena de
insert por escena es un swap parcial de la voz. Si el swap de sesión conserva
lo que no cambió, 11.18 sale gratis como caso particular.

### ¿Qué se rompe que no estás viendo?

1. El contador de compás (arriba). Afecta a todo el corpus, no solo al live.
2. El flam del swap por granularidad de bloque.
3. La cola perdida.
4. La validación salteada en el camino rápido.
5. `find_track(...)` devolviendo `None` en `apply_change` es un no-op
   silencioso más. No puede pasar hoy porque el diff exige la misma cantidad de
   tracks, pero es exactamente la forma que tuvieron los otros ocho.
6. Dos implementaciones del swap si la CLI copia la del WASM.
7. Un swap con `pending` ya encolado: si llega una edición de parámetro
   mientras hay un motor esperando el compás, el diff se hace contra el AST
   más nuevo pero el cambio se aplica al motor viejo, con índices del nuevo.
   Puede mover el `level` del track equivocado. En el navegador es difícil de
   provocar (hay que editar dos veces en un compás); en `watch` con un editor
   que guarda al tipear, es lo normal.

### ¿Hay un camino más corto?

Sí: **no diseñar un lenguaje de performance**. Lo que hace corto a un archivo
live no es sintaxis nueva, es no tener que escribir el rig ni el arreglo. Un
`use` y el modo sin escenas lo dan. Todo lo demás (qué suena, qué patrón, qué
nivel, qué filtro) ya se escribe en una línea por track.

Lo que no es más corto de lo que parece: `play` sin arreglar el swap. Se
puede tener `synth play` en una tarde con cpal, pero la pregunta real
("¿se sostiene media hora tocando?") la contesta el swap, no el `play`. Con el
swap de hoy la respuesta es no: cada edición corta la cola, flamea el bombo y
corre la grilla.

## Diseño propuesto para el primer paso

### `core/src/live.rs`

Dos objetos, porque en nativo viven en hilos distintos y en WASM en el mismo:

- **`LivePlanner`** (hilo de control): recibe el fuente, *siempre* parsea y
  compila (validación completa), compara con el AST anterior y produce un
  `Plan`:
  - `Unchanged`: texto idéntico.
  - `Fast(Vec<FastOp>)`: cambios de parámetro, ya resueltos a índices
    (`TrackLevel { track: 2, level }`, `ModuleParam { instrument: 0, id, value }`).
    Sin strings, sin lookups en el hilo de audio. Si algo no resuelve, es un
    swap, nunca un descarte.
  - `Swap { engine, inherit, crossfade }`: motor nuevo ya construido, más un
    mapa de qué hereda del viejo: sends si `delay`/`reverb`/retornos son
    iguales; master si la cadena es igual; cada bus por nombre si su cadena es
    igual; cada instrumento por nombre si su definición es igual; cada track
    por nombre si su definición, su patrón, su instrumento y la escala son
    iguales y las escenas no cambiaron. `crossfade` solo si algo del viejo
    desaparece.
- **`LivePlayer`** (hilo de audio): tiene el motor, el `pending` y el
  crossfade. `process` parte el bloque en la muestra exacta del compás. En el
  swap, el motor nuevo toma con `mem::swap` el estado que hereda (cero
  reservas), los tracks que no siguen sueltan sus notas en el viejo para que
  un instrumento heredado no quede con una nota que nadie va a soltar, y el
  motor viejo se devuelve al llamador para que lo tire fuera del hilo de audio.
  Un `Fast` que llega con un `pending` encolado se aplica al `pending`, que es
  a quien corresponden sus índices.

El WASM queda como una cáscara sobre los dos. La API de `Synth` no cambia.

### CLI

```
synth play  song.synth [--device <nombre>]     # toca hasta que termine el arreglo o 'q'
synth watch song.synth [--device <nombre>]     # play + recarga al guardar; un error imprime y sigue sonando
```

cpal 0.18 (verificado que compila y abre el dispositivo en esta máquina; el
default acepta 44.1 kHz, que es lo único que el motor sabe hacer). Un hilo
mira el `mtime` cada 50 ms, planifica, y manda el plan por un
`sync_channel`; el callback hace `try_recv`, aplica y devuelve lo retirado.
Al salir imprime el peor bloque contra el presupuesto y cuántos swaps hubo:
esa es la medida de "se sostiene".

### Tests que hay que escribir

- El compás cruza en la muestra exacta; un cambio de escena no acorta el step
  15; un render con arreglo tiene todas sus muestras.
- Un swap estructural a un tema idéntico da un render **bit a bit igual** al
  render sin swap (con herencia total no debería haber diferencia; hoy hay
  7 dB).
- Un swap con un patrón cambiado no toca la cola de la reverb: la
  `reverb_return` medida sola es igual antes y después.
- Ningún bombo doble en el compás del swap.
- `cutoff 7.0` por el camino rápido es error de compilación, igual que en
  `check`.
- El swap del `LivePlayer` con el asignador que cuenta: cero reservas.

### Orden

1. Fix del compás con sus tests. Commit solo.
2. `live.rs` con herencia y split exacto. WASM sobre él. Commit.
3. `synth play` / `synth watch`. Commit.
4. Soak: `watch` con un script que edite el archivo cada dos segundos durante
   varios minutos, con auriculares. Peor bloque y swaps, en el commit.
5. `use "rig.synth"`, lints que respeten el modo live, `docs/DSL.md`.

Lo que dejo fuera a propósito: entrada MIDI, resampleo a 48 kHz, la Pi. Nada
de eso cambia el diseño; todo depende de que el swap sea correcto primero.
