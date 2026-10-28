# Cómo contribuir

Gracias por querer mejorar Snapclone. Esta guía explica cómo preparar el entorno, cómo proponer cambios y qué se revisa antes de aceptarlos.

## Informar de un fallo o proponer una mejora

Abre una [issue](https://github.com/yosnap/snapclone/issues) antes de empezar un cambio grande, para hablarlo primero. Si es un fallo, incluye:

- la versión de Snapclone (está en el nombre del `.dmg` y en Snapclone.app → Obtener información) y la de macOS;
- si tu Mac es Apple Silicon o Intel, y cuántas pantallas usas;
- los pasos para reproducirlo, qué esperabas y qué ha pasado;
- si se cierra sola, el informe de fallo de Consola → Informes de fallos → `Snapclone`.

Los fallos de seguridad no van en issues públicas: sigue [SECURITY.md](SECURITY.md).

## Preparar el entorno

Necesitas macOS 13 o posterior y:

- [Node](https://nodejs.org) 20 o posterior, con npm;
- [Rust](https://rustup.rs) estable;
- Xcode Command Line Tools: `xcode-select --install`.

```sh
git clone https://github.com/yosnap/snapclone.git
cd snapclone
npm install
npm run tauri:dev
```

`npm run tauri:dev` arranca **Snapclone Dev**, con su propio identificador (`com.paulo.snapclone.dev`). Así sus ajustes y su permiso de Grabación de pantalla no se mezclan con los de la versión instalada, y puedes tener las dos abiertas a la vez. Si las dos usan los mismos atajos, cambia los de la versión de desarrollo en sus Ajustes.

La primera vez, macOS pedirá el permiso de Grabación de pantalla para Snapclone Dev.

## Estructura

- `src/`: frontend en TypeScript, sin framework, compilado con Vite. Hay una página por ventana: editor, selector, ajustes, HUD y scroll.
- `src-tauri/src/`: backend en Rust con Tauri 2. Incluye la captura, el selector nativo, OCR con Vision, el portapapeles, los atajos globales y el historial.
- `scripts/`: el script de release y el generador de licencias de terceros.
- `docs/`: documentación para mantenedores.

## Proponer un cambio

1. Haz un fork y crea una rama desde `main` con un nombre descriptivo, por ejemplo `fix/atajo-ventana` o `feat/flecha-curva`.
2. Haz commits pequeños con [Conventional Commits](https://www.conventionalcommits.org/es/): `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `build:` o `chore:`, seguidos de una descripción en español.
3. Antes de abrir la pull request, comprueba que todo pasa:
   ```sh
   npm run build
   cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
   ```
4. Si añades o actualizas dependencias, regenera el aviso de licencias y haz commit del resultado:
   ```sh
   npm run notices
   ```
5. Abre la pull request contra `main`. Explica qué cambia, por qué y cómo lo has probado. Si toca la interfaz, añade una captura.

## Normas del código

- Sigue el estilo del código que tengas alrededor: nombres, comentarios e idioma.
- Los ficheros no deben pasar de unas 1000 líneas. Si crecen, divídelos en módulos.
- Añade tests para la lógica nueva de Rust que no dependa de la pantalla.
- No subas secretos, ficheros `.env`, certificados ni datos personales.
- No incluyas código, iconos, imágenes ni textos de otras apps cuya licencia no lo permita.
- Interfaz: nada de bordes de color solo en el lado izquierdo de avisos y tarjetas. Distingue el tipo de aviso con el fondo, el texto, el icono o un borde completo.

## Licencia de las contribuciones

Snapclone usa la licencia [MIT](LICENSE). Al enviar una contribución, aceptas que se publique con esa misma licencia y confirmas que tienes derecho a hacerlo.

## Convivencia

Trata a los demás con respeto, en las issues y en las revisiones. Las críticas van dirigidas al código, nunca a las personas.
