

# Kinjo

<div align="center">

**Explora servicios DNS-SD locales, fíltralos y ejecuta acciones personalizadas desde una interfaz de terminal.**

![GitHub Release](https://img.shields.io/github/v/release/abbyssoul/kinjo?display_name=tag&color=%23a6a)
[![Crates.io](https://img.shields.io/crates/v/kinjo.svg)](https://crates.io/crates/kinjo)
[![docs.rs](https://img.shields.io/docsrs/kinjo)](https://docs.rs/kinjo)
[![GitHub branch check runs](https://img.shields.io/github/check-runs/abbyssoul/kinjo/main)](https://github.com/abbyssoul/kinjo/actions/workflows/ci-test.yml)
[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=abbyssoul_kinjo&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=abbyssoul_kinjo)
[![License: MIT](https://img.shields.io/github/license/abbyssoul/kinjo)](LICENSE)

<img width="1430" height="609" alt="kinjo-screenshot" src="https://github.com/user-attachments/assets/319acde6-a3d0-4cb1-aefc-12088bc67328" />


</div>

Kinjo convierte los servicios ya anunciados en tu red local en un lanzador útil controlado por teclado. Encuentra un servidor SSH, la interfaz web de un dispositivo, un servicio de desarrollo o una impresora sin memorizar nombres de host y puertos.

- Explora servicios por nombre, host, tipo o comando coincidente.
- Filtra una red saturada con búsqueda difusa y filtros por tipo de servicio.
- Ejecuta acciones configurables como SSH o abrir una interfaz web.
- Descubre mediante mDNS/DNS-SD estándar: no se requiere una CLI de descubrimiento externa.
- Mantén todo local: no hay telemetría, cuentas ni servicios en la nube.

Kinjo es compatible con **macOS, Linux y Windows**. El backend de descubrimiento predeterminado funciona directamente; Avahi solo es necesario si compilas y seleccionas deliberadamente el backend opcional `zeroconf`.

## Instalación

Elige la opción más conveniente para tu plataforma:

| Plataforma | Instalación recomendada |
|---|---|
| macOS | [Homebrew](#homebrew-macos-y-linux) |
| Debian / Ubuntu | [paquete `.deb`](#debian--ubuntu) o [Homebrew](#homebrew-macos-y-linux) |
| Otro Linux | [Homebrew](#homebrew-macos-y-linux) o [Nix](#nix--nixos) |
| Windows | [Cargo](#cargo-avanzado) o [compilar desde el código fuente](#compilar-desde-el-código-fuente) |

### Homebrew (macOS y Linux)

Instala Kinjo directamente desde el tap
[`abbyssoul/abyss`](https://github.com/abbyssoul/homebrew-abyss):

```sh
brew install abbyssoul/abyss/kinjo
```

Ese único comando agrega el tap, instala Kinjo e incluye los comandos predeterminados. Las futuras versiones estarán disponibles mediante el habitual `brew upgrade`.

### Debian / Ubuntu

Descarga el `.deb` para tu arquitectura (`amd64` o `arm64`) desde la
[última versión de GitHub](https://github.com/abbyssoul/kinjo/releases/latest),
luego instálalo con `apt`:

```sh
sudo apt install ./kinjo_*.deb
```

El paquete incluye el binario `kinjo` y los comandos predeterminados. El backend
de descubrimiento predeterminado no requiere `avahi-daemon` ni encabezados de desarrollo.

### Nix / NixOS

Ejecuta Kinjo sin instalarlo:

```sh
nix run github:abbyssoul/kinjo
```

El flake proporciona paquetes para `x86_64-linux` y `aarch64-linux`. Para instalar
Kinjo de forma declarativa, agrega el flake como entrada y usa su overlay:

```nix
# flake inputs:
kinjo.url = "github:abbyssoul/kinjo";

# in your NixOS module:
nixpkgs.overlays = [ inputs.kinjo.overlays.default ];
environment.systemPackages = [ pkgs.kinjo ];
```

La compilación de Nix utiliza el backend predeterminado `mdns-sd`, por lo que no
hay ningún demonio que habilitar en NixOS.

### Cargo (avanzado)

Si ya tienes el [kit de herramientas de Rust](https://rustup.rs/), instala Kinjo
desde crates.io:

```sh
cargo install kinjo
```

Funciona en macOS, Linux y Windows y compila Kinjo localmente. Windows se prueba
en CI pero actualmente no tiene una instalación mediante gestor de paquetes, por lo
que Cargo es la opción más simple para Windows.

Los backends de descubrimiento opcionales se seleccionan en tiempo de compilación. Por ejemplo, el
backend `zeroconf` respaldado por Avahi en Linux requiere los encabezados de desarrollo del cliente Avahi:

```sh
cargo install kinjo --features zeroconf
```

### Compilar desde el código fuente

Clona el repositorio y compila un binario de lanzamiento:

```sh
git clone https://github.com/abbyssoul/kinjo.git
cd kinjo
cargo build --release --locked
```

El binario estará en `target/release/kinjo` (`target\release\kinjo.exe` en
Windows). Para instalarlo en el directorio bin de Cargo en su lugar:

```sh
cargo install --path .
```

Los colaboradores pueden encontrar la configuración de desarrollo completa y los comandos de verificación en
[CONTRIBUTING.md](CONTRIBUTING.md).

## Prueba

Inicia Kinjo sin configuración para explorar el dominio predeterminado `local`:

```sh
kinjo
```

Usa las teclas de flecha o `j`/`k` para moverte, `/` para buscar, `tab` para cambiar de vista,
`enter` para ejecutar una acción coincidente y `?` para ver todos los atajos. Presiona `q` para
salir.

Explora otro dominio DNS-SD con `--domain` (`-d`):

```sh
kinjo --domain example.local
```

Desde un clon del código fuente, puedes explorar un conjunto determinista de servicios
de ejemplo incluso cuando no haya nada anunciándose en tu red:

```sh
cargo run --features fake -- --backend fake --config-dir actions
```

## Backends de descubrimiento

La aplicación descubre servicios mediante mDNS/DNS-SD, por lo que no se requieren
herramientas CLI externas. Los backends se seleccionan exclusivamente con `--backend`:

- `mdns-sd` (predeterminado): el crate `mdns-sd-discovery`. Un único navegador
  enumera cada tipo de servicio en el enlace mediante la metaconsulta DNS-SD nativa.
  Acepta un `--domain` personalizado.
- `zeroconf`: el crate `zeroconf-tokio`, que se comunica con el demonio Avahi del sistema
  en Linux. Explora un tipo de servicio a la vez, por lo que se recorre en paralelo un conjunto curado de tipos
  comunes cuando no se proporciona `--service-type`. Este backend está
  detrás de la característica de cargo `zeroconf`, desactivada por defecto (requiere los encabezados del cliente Avahi
  para compilar, por ejemplo, `libavahi-client-dev` en Debian/Ubuntu).
- `fake`: un flujo de muestra incorporado y finito para desarrollo y pruebas de humo deterministas de la interfaz.
  Está detrás de la característica de Cargo `fake`, desactivada por defecto, para que los binarios
  de lanzamiento no incluyan puntos finales de muestra ejecutables a menos que se soliciten explícitamente.

```sh
cargo install kinjo --features zeroconf
kinjo --backend zeroconf
```

Instala o compila con descubrimiento de muestra explícitamente:

```sh
cargo install kinjo --features fake
kinjo --backend fake
```

Seleccionar un backend opcional en un binario compilado sin él fallará con un error
que indica la característica de Cargo que debe habilitarse.

`--fake-discovery` ha sido eliminado; reemplázalo con `--backend fake`. Consulta las
[notas de lanzamiento](docs/release-notes.md) para la decisión de migración.

**El backend `zeroconf` solo explora el dominio `local` predeterminado.** Su navegador
no expone ninguna forma de seleccionar un dominio, por lo que, en lugar de aceptar `--domain` y explorar `local` en silencio de todos modos, rechaza la combinación directamente:

```console
$ kinjo --backend zeroconf --domain corp
error: invalid value for `--domain`: the `zeroconf` backend cannot browse the
`corp` domain: it can only browse the default `local` domain. Browse `local`, or
select the `mdns-sd` backend, which supports custom domains
```

Usa el backend predeterminado `mdns-sd` para explorar un dominio personalizado. Vacío, `local` y
`local.` nombran todos el dominio predeterminado.

### Limitar el descubrimiento a un tipo de servicio

Cuando se proporciona un `--service-type`, solo se explora ese tipo:

```sh
kinjo --service-type _ssh._tcp
```

El valor debe ser un tipo de servicio DNS-SD: `_<name>._tcp` o `_<name>._udp`,
donde `<name>` tiene entre 1 y 15 caracteres ASCII (letras, dígitos y guiones internos), comienza y termina con un carácter alfanumérico y contiene al menos una letra. Los tipos de servicio no distinguen mayúsculas de minúsculas, por lo que `_SSH._TCP` y `_ssh._tcp` son la misma búsqueda.

Un valor que no sea un tipo de servicio se rechaza antes de que comience el descubrimiento, en lugar de ignorarse a favor de explorar todo: un filtro está ahí para reducir lo que el programa observa, por lo que un error tipográfico nunca debe ampliarlo:

```console
$ kinjo --service-type bogus
error: invalid value for `--service-type`: `bogus` is not a DNS-SD service type:
a service type begins with `_`. Use a type such as `_ssh._tcp` or `_dns-sd._udp`,
or omit it to browse every service type
```

Omite `--service-type` para explorar cada tipo compatible.

El descubrimiento nunca vuelve a los registros de muestra. Si el descubrimiento mDNS no está disponible,
la lista permanece vacía y la línea de estado explica por qué. Si una exploración en ejecución se detiene inesperadamente, `kinjo` lo indica y limpia la lista en lugar de dejar entradas obsoletas en pantalla: mDNS es activado por eventos, por lo que una vez que la exploración desaparece, nada puede informar que un servicio listado ha desaparecido desde entonces, y un comando iniciado podría apuntar a un host que ya no está allí. En cualquier caso, el error y su causa permanecen en pantalla en lugar de desplazarse hacia atrás: el descubrimiento no se reintenta automáticamente. Actualizar (`r`) lo reinicia y es la acción de recuperación ante un fallo.

Selecciona `--backend fake` en una compilación con la característica `fake` para registros de muestra bajo demanda. Esas muestras son un flujo corto y finito; cuando termina, la línea de estado informa la finalización normal y las muestras permanecen listadas.

El conjunto de muestras se eligió para probar el comportamiento de la aplicación real: un servicio
accesible en varias direcciones, un servicio que aún no tiene un host resuelto y SSH en
dos hosts diferentes: por lo que la fila de tipo de servicio `_ssh._tcp` agrega elementos hijos cuyos comandos difieren y pregunta sobre qué host actuar.

## Privacidad

`kinjo` explora tu red local, por lo que vale la pena ser explícito sobre lo que
hace y lo que no hace con ese acceso:

- **Sin telemetría.** `kinjo` no llama a casa, recopila analíticas ni envía
  datos de uso a ningún lugar. No hay verificador de actualizaciones ni informe de fallos.
- **Sin exploración proactiva.** `kinjo` nunca escanea puertos ni sondea hosts por iniciativa
  propia. Todo el descubrimiento se delega a un backend intercambiable detrás de
  una sesión de descubrimiento (ver [Arquitectura](#arquitectura)), y cada backend
  solo habla el protocolo estándar mDNS/DNS-SD: muestra los servicios que
  ya se están anunciando en el enlace, nada más.
- **Los datos descubiertos permanecen locales.** Los servicios encontrados en la red se muestran en
  la terminal y se utilizan únicamente para completar los comandos que configuras. Nada se
  carga ni comparte con nadie más que contigo.

Los dos backends de descubrimiento de red difieren en cómo acceden a la red:

- `mdns-sd` (predeterminado) implementa mDNS/DNS-SD por sí mismo: envía
  consultas multicast estándar en el enlace local y escucha las respuestas. No realiza
  otras llamadas de red ni se comunica con nada fuera del enlace.
- `zeroconf` (opcional, detrás de la característica de cargo `zeroconf`) delega toda
  la E/S de red al demonio `avahi-daemon` del sistema mediante D-Bus. `kinjo` en sí no abre
  sockets en este modo: solo lee los registros que el demonio ya
  mantiene.

En cualquier caso, el tráfico involucrado es del mismo tipo de consulta multicast local
que tu sistema operativo ya realiza para el descubrimiento Bonjour/AirPlay/impresoras de red, no
un escaneo de red general.

## Cómo Funciona

`kinjo` tiene cinco componentes móviles:

1. El descubrimiento encuentra registros de servicio DNS-SD en la red. Cada registro puede llevar
   campos como nombre de servicio, tipo de servicio, dominio, nombre de host, dirección, puerto,
   y valores TXT.
2. La filtración y las pestañas de vista organizan esos registros en la interfaz. Puedes buscar de forma difusa
   los servicios visibles, limitar por tipo de servicio y cambiar la pestaña del panel superior
   para ver el descubrimiento por servicio, host, tipo de servicio o comando coincidente.
3. Las acciones deciden qué se puede hacer con un servicio seleccionado. Los archivos de comandos de acción
   definen predicados de coincidencia como "el tipo de servicio es igual a `_ssh._tcp`" o "el campo TXT
   contiene una URL".
4. Las plantillas de comando convierten los campos de servicio en comandos ejecutables. Por ejemplo,
   `ssh {hostname}` usa el nombre de host del servicio seleccionado, mientras que
   `xdg-open http://{hostname}:{port}` construye una URL a partir de la instancia seleccionada.
5. Los atajos de teclado controlan la interfaz TUI. Los valores predeterminados usan navegación al estilo Vim, y cada
   comando de interfaz integrado se puede reasignar en `keybindings.toml`.

El resultado es un explorador de servicios locales pequeño que se comporta como un lanzador configurable: descubre servicios, reduce la lista, elige una acción coincidente y ejecuta
el comando construido a partir de los campos de ese servicio.

### Arquitectura

Internamente, esos componentes viven en tres módulos deliberadamente desacoplados, para
que el proyecto sea fácil de extender y modificar. Cada uno está diseñado para ser intercambiado o
reutilizado de forma independiente:

1. **Descubrimiento** (`src/discovery/`) — el productor de *entradas*. Una entrada es un
   registro descubierto descrito enteramente por sus atributos (nombre, tipo, host,
   dirección, puerto, TXT, …); `Entry` es el único contrato del que depende el resto del programa.
   Iniciar el descubrimiento devuelve una `DiscoverySession`: un valor
   que posee el adaptador en ejecución, sus eventos, su estado y su cierre, por lo que
   el llamador no puede retener un receptor cuyo productor haya muerto en silencio, y descartarlo
   detiene la exploración. Los adaptadores varían detrás de esa sesión: el backend mDNS/Avahi
   es el predeterminado, con un backend de muestra incorporado protegido por características seleccionado
   mediante `--backend fake`, por lo que una fuente DNS-SD diferente, un
   archivo estático o un
   explorador SSDP/UPnP se ajusta junto a ellos sin perturbar nada por encima
   (ver [Extenderlo](#extenderlo) para ver dónde está esa costura). Las opciones de descubrimiento se
   verifican una vez en esa costura: un `DiscoveryConfig` es una solicitud, y validarla
   produce las `DiscoveryOptions` que iniciar un adaptador requiere. Un
   tipo de servicio mal formado, o un dominio que el backend seleccionado no puede cumplir, es
   rechazado antes de que se inicie nada: ningún adaptador puede ser alcanzado con un
   valor que tendría que reinterpretar en silencio.
2. **Plumber** (`src/plumber/`) — el motor de reglas. Una colección serializable de
   reglas de comando (los archivos TOML de comando) se coincide con las entradas por sus
   atributos; múltiples reglas pueden coincidir con una entrada, y una regla coincidente puede
   ejecutarse. Depende solo de `Entry`, nunca de la interfaz, y está detrás del
   trait `RuleEngine` para que se pueda sustituir una estrategia de coincidencia alternativa.
3. **Interfaz** (`src/ui/`) — une el descubrimiento y el motor de reglas para una persona
   en la terminal: análisis CLI, carga de configuración y mapa de teclado, la máquina de estados de la aplicación
   y el renderizado. Depende de los otros dos; ellos no dependen
   de ella. `App` es el bucle de eventos y el estado del que decide; su interfaz es
   seis operaciones — construirla, adjuntar una fábrica de descubrimiento y un cargador de configuración,
   entregar su desencadenante de recarga, ejecutarla y tomar cualquier diagnóstico de recarga que
   haya sobrevivido a la terminal. Su estado no es público. El renderizado es una función pura
   de él (`&App` entra, un fotograma sale), y el límite aplicación/renderizado es la visibilidad de campos en lugar de una vista proyectada: ver
   [ADR 0002](docs/adr/0002-render-reads-the-app-directly.md).

El flujo de dependencias es unidireccional — `discovery ← plumber ← ui` — cableado
juntos por `run` en `src/lib.rs`, del cual el binario `kinjo` es un envoltorio delgado.

### Extenderlo

Las dos capas son extensibles de formas deliberadamente diferentes, y la diferencia
es la parte interesante:

- **Un motor de reglas es sustituible desde el exterior.** `RuleEngine` es público, y
  `App::new` acepta cualquier implementador, por lo que un crate que dependa de `kinjo` puede traer
  su propia estrategia de coincidencia. `Matcher` es el motor que envía `kinjo`, no el único
  que admite la interfaz. Hacerlo significa escribir tu propia raíz de composición:
  `run` carga un `Matcher` y lo usa concretamente, por lo que no es genérico sobre el
  motor: haz lo que hace `run`, con tu motor en su lugar. Ver
  [ADR 0001](docs/adr/0001-rule-engine-is-a-supported-extension-point.md).
- **Un backend de descubrimiento no lo es.** Los adaptadores varían *dentro* de `src/discovery/`, en
  el bucle de exploración, detrás de una `DiscoverySession` concreta y una enumeración `DiscoveryBackend` cerrada.
  Difieren en cómo exploran pero no en cómo un llamador los ejecuta y detiene, por lo que no hay un trait que implementar: una nueva fuente —
  un archivo estático, un explorador SSDP/UPnP — es un módulo y una variante de enumeración en este
  crate, no algo que una dependencia agregue desde el exterior.

Consulta [CONTRIBUTING.md](CONTRIBUTING.md) para la configuración de desarrollo.

## Interfaz de Usuario (UI)

Las teclas predeterminadas siguen convenciones al estilo Vim:

- `j` / `flecha abajo`: mover hacia abajo
- `k` / `flecha arriba`: mover hacia arriba
- `enter`: mostrar o ejecutar acciones coincidentes
- `/`: filtro de texto difuso
- `t`: filtro de lista de verificación de tipos de servicio
- `tab` / `shift+tab` (o `←` / `→`): cambiar pestaña de vista
- `s`: reducir la lista al host de la fila seleccionada (presionar de nuevo para limpiar)
- `d` / `u` (o `página abajo` / `página arriba`): desplazarse por el panel de detalles
- `r` / `F5`: actualizar — reiniciar el descubrimiento de servicios desde cero
- `?`: ayuda
- `q`: salir

El panel superior expone cuatro pestañas: servicios, hosts, tipos y comandos. Cada pestaña
intercambia los paneles de lista y detalles para ver el descubrimiento desde ese ángulo: servicios
individuales, hosts y los servicios que ofrecen, tipos de servicio descubiertos o
comandos configurados y los servicios con los que coinciden. La lista también admite búsqueda de texto
difusa y filtrado por tipo de servicio.

Cada pestaña muestra cuántas filas enumera: servicios lógicos, hosts (más una fila para
los registros que aún no han resuelto un host), tipos de servicio distintos o
comandos configurados. Las pestañas agregadas informan solo lo que es cierto de una fila completa.
Los detalles de un host nombran al host y listan cada servicio en él: cada uno con su propio
tipo, puerto y datos TXT, en lugar de presentar los campos de un servicio como los
del host. Los detalles de un tipo de servicio listan de igual manera cada host que lo ofrece. Las acciones
siempre se ejecutan contra un servicio descubierto concreto, desde la pestaña que sean.

El indicador en la esquina superior izquierda indica qué está haciendo el descubrimiento, y solo se anima
mientras algo esté sucediendo realmente:

| Indicador | Significado |
|---|---|
| `⠋` (girando) | Explorando. La lista aún puede cambiar. |
| `✓` | Un flujo de muestra terminó normalmente. Sus registros permanecen válidos; no llegará nada más. |
| `✗` | El descubrimiento falló o se detuvo. Sus registros ya no se confirman. |

Un indicador estático significa, por lo tanto, que la lista es final hasta que actualices (`r`),
lo cual inicia una nueva exploración y devuelve el indicador a girar. El mensaje de lista vacía dice lo mismo en palabras, por lo que los dos nunca pueden contradecirse. Ningún final reintenta por sí mismo.

El filtro de tipo (`t`) lista los tipos de servicio que se anuncian actualmente, y
su chip `types n/m` cuenta solo esos: `m` es cuántos tipos hay en el enlace ahora mismo, y `n` es cuántos de ellos estás mostrando. Desactivar un tipo se recuerda en lugar de observarse, por lo que un dispositivo que se desconecta del enlace y vuelve a conectarse permanece desactivado: pero un tipo que nadie está anunciando no se cuenta en ningún lado del chip.

El panel de detalles mantiene tu posición de desplazamiento mientras permaneces en la misma fila,
incluso mientras el descubrimiento lo reinforma. Moverse a una fila diferente inicia sus detalles desde arriba, y una fila que se acorta o una terminal que crece devuelve la vista al final del contenido en lugar de más allá.

El filtro `s` (mismo-host) necesita una fila con un solo host, por lo que está disponible en
las pestañas de servicios y hosts. Las pestañas de tipos y comandos informan que no está disponible en lugar de adivinar un host; un filtro activo se puede limpiar desde cualquier pestaña.

Los atajos de teclado son totalmente personalizables: todos los comandos de interfaz integrados se pueden reasignar con
un archivo de configuración de atajos. Consulta [docs/keybindings.md](docs/keybindings.md) para
la referencia completa de atajos y ejemplos.

## Configuración

Los archivos de comando siguen la especificación XDG Base Directory. Los archivos de comando de usuario se
cargan desde:

```sh
$XDG_CONFIG_HOME/kinjo/commands/*.toml
```

Si `XDG_CONFIG_HOME` no está configurado, la ruta de respaldo es:

```sh
~/.config/kinjo/commands/*.toml
```

Se pueden proporcionar directorios de comandos adicionales con:

```sh
kinjo --config-dir ./commands
```

Valida y lista los comandos registrados con:

```sh
kinjo list-commands
```

Para validar y listar solo los comandos de un directorio específico:

```sh
kinjo list-commands --config-dir ./commands
```

`--config-dir` puede escribirse a cualquiera de los lados de `list-commands`, y repetirse en
ambos; las dos líneas a continuación son equivalentes. Los directorios siempre se superponen en el
orden en que aparecen en la línea de comandos, sin importar en qué lado se escribieron.

```sh
kinjo --config-dir ./commands list-commands
kinjo list-commands --config-dir ./commands
```

Una instancia en ejecución recarga sus archivos de comando con `SIGHUP` (la señal de recarga convencional), por lo que los cambios se aplican sin reiniciar la interfaz TUI:

```sh
pkill -HUP kinjo
```

La recarga es transaccional: se aplica solo si cada archivo de comando configurado es
válido. Si uno no lo es, los comandos ya cargados permanecen en vigor y se informa la razón, en lugar de que una edición guardada a medias quite un comando que estabas usando. Ver
[docs/actions.md](docs/actions.md#reloading-while-it-runs).

Los atajos de teclado se pueden anular en:

```sh
$XDG_CONFIG_HOME/kinjo/keybindings.toml
```

Consulta [docs/keybindings.md](docs/keybindings.md) para ejemplos y la lista completa
de comandos asignables.

## Archivos de Comando

Cada archivo de comando define una acción y predicados de coincidencia estructurados. Ejemplo de abre SSH:

```toml
[metadata]
name = "ssh"
description = "SSH into a service"
requirements = ["ssh"]

[match.service_type]
equals = "_ssh._tcp"

[action]
description = "SSH into the selected service"
command = "ssh -- {hostname}"
mode = "execute"
```

Modos de acción compatibles:

- `fork`: generar el comando y volver a la interfaz TUI.
- `execute`: restaurar la terminal y reemplazar el proceso TUI con el comando.

Predicados de coincidencia compatibles:

- `equals`
- `contains`
- `regex`

Campos de servicio compatibles:

- `name`
- `service_type` o `type`
- `domain`
- `hostname`
- `address`
- `port`
- `txt.<key>`

Los mismos campos se pueden usar en la interpolación de comandos de acción, por ejemplo
`{hostname}`, `{address}` y `{port}`.

Múltiples acciones configuradas pueden coincidir con el mismo servicio. En ese caso, la interfaz TUI
muestra un selector de acciones. Si una acción necesita campos específicos de la instancia, como
`address` o `port`, y la fila seleccionada contiene múltiples instancias, la interfaz TUI
pregunta qué instancia exacta usar.

Para el formato completo del archivo de comando, ejemplos y reglas de superposición, consulta
[docs/actions.md](docs/actions.md).

## Contribuir

Los informes de errores, ideas de características, correcciones de documentación y solicitudes de extracción son bienvenidos.
Si no estás seguro por dónde empezar, abre un
[issue](https://github.com/abbyssoul/kinjo/issues) o consulta
[CONTRIBUTING.md](CONTRIBUTING.md) para la configuración de desarrollo y los comandos de verificación local.
