# Snapclone

Snapclone es una app de barra de menús para macOS que hace capturas de pantalla y las deja listas para compartir: fondo, margen, esquinas, sombra, anotaciones y tachado automático de datos sensibles.

> Estado: versión 0.x, en desarrollo activo. Hasta la 1.0 puede haber cambios de comportamiento entre versiones.

## Funciones

- Captura de zona, ventana, pantalla completa y con scroll, con atajos configurables.
- «Capturar zona y copiar»: copia el resultado al portapapeles sin abrir el editor.
- Editor con fondo (degradados o tu fondo de escritorio difuminado), margen, inset, esquinas, sombra, proporciones y tamaños para redes sociales.
- Anotaciones: flecha, rectángulo, texto y pixelado o tapado.
- Tachado automático de correos, tarjetas, IP y claves, gracias al reconocimiento de texto de Vision.
- Historial de capturas, texto detectado, abrir imágenes desde un archivo o el portapapeles, arrastrar y compartir.

## Requisitos

- macOS 13 (Ventura) o posterior, en Apple Silicon o Intel.
- Permiso de **Grabación de pantalla** (Ajustes del Sistema → Privacidad y seguridad). Snapclone lo pide la primera vez que haces una captura.

## Instalación

1. Descarga `Snapclone_<versión>_universal.dmg` de la [última release](https://github.com/yosnap/snapclone/releases).
2. Abre el `.dmg` y arrastra Snapclone a Aplicaciones.
3. Abre Snapclone. Aparece en la barra de menús.

## Desarrollo

Requisitos: Node 20 o posterior, Rust estable y Xcode Command Line Tools.

```sh
npm install
npm run tauri dev
```

Comprobaciones antes de fusionar:

```sh
npm run build
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
```

El procedimiento de release está en [docs/release.md](docs/release.md).

## Licencia

[MIT](LICENSE). Snapclone es un proyecto independiente, sin relación con Xnapper ni con sus autores.
