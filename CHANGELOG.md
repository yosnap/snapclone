# Changelog

Todos los cambios relevantes de Snapclone. El formato sigue [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/) y las versiones siguen [SemVer](https://semver.org/lang/es/). Mientras la versión sea 0.x, cualquier versión menor puede cambiar el comportamiento.

## [0.1.1] - 2026-10-31

### Corregido

- La app se cerraba de golpe al mover el puntero de un monitor a otro con el selector de zona abierto, sobre todo con «Capturar zona y copiar». macOS 27 avisa de la salida del puntero sin comprobar si el cursor en cruz sabe atenderla.

## [0.1.0] - 2026-10-28

Primera versión pública, solo para macOS.

### Captura

- Selector de zona propio en todos los monitores, con modo ventana (Espacio), puntero en cruz y medidas en píxeles.
- Captura de ventana, de pantalla completa y con scroll, que une los fotogramas detectando el solapamiento.
- «Capturar zona y copiar» (⌃⌥⌘5): copia al portapapeles sin abrir el editor y muestra un aviso breve.
- Atajos globales configurables para cada tipo de captura.

### Editor

- Fondo con degradados, transparente o con el fondo de escritorio difuminado; margen, inset, esquinas, sombra y equilibrado del contenido.
- Proporciones y tamaños para redes sociales.
- Presets de estilo con nombre y ajustes persistentes.
- Anotaciones: flecha con contorno, rectángulo, texto y pixelado o tapado sólido, con deshacer.
- Tachado automático de correos, tarjetas, IP y claves con Vision, que respeta lo que cambies a mano.
- Vista de texto detectado con «Copiar todo».

### Biblioteca y salida

- Historial de capturas en SQLite, con su estilo y sus anotaciones.
- Abrir imágenes desde un archivo (⌘O), desde el portapapeles (⌘V) o arrastrándolas al editor.
- Reabrir la última captura (⇧⌘Z) y copiar su texto desde la barra de menús.
- Exportar, copiar, guardar con nombre único, arrastrar fuera y compartir.

### App

- Vive en la barra de menús. ⌘Q la oculta y «Salir del todo» la cierra.
- Ventana de Preferencias con opción para ocultar el Dock.

[0.1.1]: https://github.com/yosnap/snapclone/releases/tag/v0.1.1
[0.1.0]: https://github.com/yosnap/snapclone/releases/tag/v0.1.0
