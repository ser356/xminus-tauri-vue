# Secretos embebidos en el binario

Todo lo que pongas en esta carpeta (salvo este README) está ignorado por git.

## Cookie de tu cuenta de X-Minus

1. Inicia sesión en <https://x-minus.pro/ai> con tu navegador.
2. Abre las herramientas de desarrollo → pestaña *Red*, recarga la página y
   selecciona la petición a `x-minus.pro/ai`.
3. Copia el valor de la cabecera de petición `Cookie` y guárdalo, en una sola
   línea, en `src-tauri/secrets/cookie.txt` (el prefijo `Cookie:` es opcional).
4. Si la cookie incluye `cf_clearance`, copia también la cabecera `User-Agent`
   del mismo navegador en `src-tauri/secrets/user_agent.txt`.
5. Compila: `npm run tauri build` (o `npm run tauri dev`).

Alternativa sin ficheros: variables de entorno `XMINUS_COOKIE` y
`XMINUS_USER_AGENT` en el momento de compilar.

La cookie queda dentro de la app solo con un ofuscado XOR, **no cifrada**:
quien tenga el binario puede extraerla y usar tu cuenta. No compartas la app
compilada con cookie. Si la cookie caduca, la app se conecta sin cuenta
(modo gratuito); vuelve a copiarla y recompila.
