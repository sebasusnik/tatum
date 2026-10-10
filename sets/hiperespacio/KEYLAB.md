# La KeyLab en hiperespacio

```
cd /Users/sebasusnik/dev/tatum/.claude/worktrees/warp
./target/release/tatum set play sets/hiperespacio --auto --tui
```

`--auto`: el set avanza solo. Sin `--auto` lo movés vos (espacio). En la TUI, **`i`** abre el monitor:
cada cosa que tocás aparece con su número y lo que hace. Todo está en `_keylab.synth`, para
cambiarlo.

## El teclado

| Octava | Notas | Qué hace |
|---|---|---|
| la más grave | 36–47 | **disparos**, una cosa por tecla (abajo) |
| la segunda | 48–59 | **bajo**: mantené una tecla y el bajo del tema la rueda en semicorcheas, en su registro; soltá y vuelve su línea |
| el resto | 60–84 | **lead**: la voz de la escena, siempre dentro de la escala |

Disparos, de izquierda a derecha:

| C | C# | D | D# | E | F | F# | G | G# | A | A# | B |
|---|---|---|---|---|---|---|---|---|---|---|---|
| riser de ruido | impacto | sin batería | sin bajo | corte de filtro | sweep | redoble | lead al eco | freeze | tape stop | toggle del tema | throw del tema |

## Las ruedas

| | Tocando el lead | Con las manos afuera del teclado |
|---|---|---|
| **pitch bend** | dobla tu nota (2 grados de la escala, o un dive de 2 octavas en el build) | dobla el bajo de la canción |
| **modulación** | cambia tu sonido: filtro, crusher o vocal según la escena | filtra la canción (o la adelgaza en el build) |

El gesto sigue en lo que empezó hasta que la rueda vuelve a cero.

## Faders

| F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 |
|---|---|---|---|---|---|---|---|---|
| kick | bajo | hats | lead | FX del tema | lead → eco | lead → sala | riser de ruido | filtro DJ |

F9, el filtro DJ: al medio está abierto, para abajo oscurece, para arriba adelgaza.

## Perillas: una página por voz

**Tab** en la compu, o el **encoder grande**, cambia de página; el encabezado de la TUI dice cuál (`knobs: bass`). Las perillas son siempre las mismas 9: la página elige qué mueven. **0** en la compu vuelve todo a como está el tema.

| página | K1 | K2 | K3 | K4 | K5 | K6 | K7 | K8 | K9 |
|---|---|---|---|---|---|---|---|---|---|
| **tema** | filtro del bajo | voz principal | firma (RPM) | segunda voz | tensión | throw | kick | hats | suciedad |
| **bass** | cutoff | reso | decay | envolvente | sustain | eco | osc 2 | vibrato | volumen |
| **lead** | brillo | reso | decay | FM | feedback | vocal (láser y pad) | crusher | caída del zap | vibrato |
| **drums** | decay kick | pitch kick | click | drive | volumen hats | pitch hats | redoblante | pan hats | toda la batería |
| **master** | volumen master | adelgazar | sala | eco | graves | medios | agudos | | |

Las perillas responden apenas las tocás. Dos esperan: el filtro DJ (F9) arranca al medio y,
si lo encontrás en otro lado, espera a pasar por el medio; el volumen master arranca arriba. Y al
volver a una página con Tab, una perilla que no está donde la dejaste espera a llegar ahí: la
pantalla dice para dónde girarla.

## Pads

| | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|
| **banco A** (efectos) | roll 1/8 | roll 1/16 | roll 1/32 | tape stop | trance gate | bitcrush | grito | freeze |
| **banco B** (el tema) | firma | riser | crash | figura | roll 1/32 | firma al eco | libre | libre |

Banco A: cuanto más fuerte golpeás, más intenso; si apretás más mientras lo mantenés, también.

## La compu

| Tecla | Qué hace |
|---|---|
| F1 · F2 · F3 · F4 | escena intro · build · drop · break (entran en el próximo compás) |
| Tab | página siguiente de perillas |
| 0 | reset: todo lo que moviste con perillas y faders vuelve a como está el tema |
| espacio o → | próximo paso del set · ← el anterior · 1–9 un paso de la pantalla · g la lista |
| i | monitor MIDI |
| ? | todas las teclas de la TUI |
| q | salir |

En la Mac las F1–F4 pueden necesitar `fn`; en el bloque `keyboard` de `_keylab.synth` se cambian
por letras.
