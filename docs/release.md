# Cómo sacar una release

Por ahora solo hay releases para macOS: un `.dmg` universal (Apple Silicon e Intel), firmado con Developer ID y notarizado. Se compila en local con `npm run release:mac`.

## Preparación, una vez por Mac

1. Instala los targets de Rust:
   ```sh
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   ```
2. Crea el certificado **Developer ID Application** desde Xcode → Ajustes → Cuentas → tu equipo → Gestionar certificados → «+». Hace falta pertenecer al Apple Developer Program.
3. Crea una contraseña de aplicación en [account.apple.com](https://account.apple.com), en Inicio de sesión y seguridad → Contraseñas de aplicaciones.
4. Guárdala en el llavero. El comando la pide sin mostrarla:
   ```sh
   security add-generic-password -a "<tu Apple ID>" -s snapclone-notary -w
   ```

El script obtiene la identidad de firma, el Team ID y el Apple ID desde el llavero. No hace falta ningún fichero `.env`.

## Versiones

- Semver. Mientras la versión sea `0.x`, todas las releases se publican como **pre-release**. La `1.0.0` solo sale cuando se dé por buena la app completa.
- `package.json` es la fuente de la versión, y `tauri.conf.json` la lee de ahí. En `Cargo.toml` hay que cambiarla a mano, y el script de release se niega a seguir si no coincide.

## Procedimiento

1. En `main`, con el árbol limpio, sube la versión:
   ```sh
   npm version 0.2.0 --no-git-tag-version
   sed -i '' 's/^version = ".*"/version = "0.2.0"/' src-tauri/Cargo.toml
   ```
2. Añade la sección de la versión al principio de `CHANGELOG.md`.
3. Haz el commit y crea la etiqueta:
   ```sh
   git commit -am "chore(release): v0.2.0"
   git tag -a v0.2.0 -m "Snapclone 0.2.0"
   ```
4. Compila, firma, notariza y verifica:
   ```sh
   npm run release:mac
   ```
   El resultado queda en `release/Snapclone_<versión>_universal.dmg` junto a su `.sha256`.
5. Publica:
   ```sh
   git push --follow-tags
   gh release create v0.2.0 --prerelease --title "Snapclone 0.2.0" \
     --notes-file <(sed -n '/^## \[0.2.0\]/,/^## \[/p' CHANGELOG.md | sed '$d') \
     release/Snapclone_0.2.0_universal.dmg release/Snapclone_0.2.0_universal.dmg.sha256
   ```

## Si algo falla

- **La notarización se rechaza:** busca el identificador del envío en la salida y consulta el detalle con `xcrun notarytool log <id> --apple-id … --team-id …`. Normalmente el motivo es un binario sin firmar o un derecho (entitlement) que no está permitido.
- **«No hay ningún certificado Developer ID Application»:** comprueba que aparece en `security find-identity -v -p codesigning`. Un certificado «Apple Development» no sirve para distribuir.
- **Tras instalar, macOS vuelve a pedir el permiso de Grabación de pantalla:** es normal. El permiso va ligado a la firma, y la build de release no es la misma app que la de desarrollo.
