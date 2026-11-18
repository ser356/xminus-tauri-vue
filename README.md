# X-Minus Stems v0.3 — Tauri 2 + Vue 3

Wrapper **no oficial** de `https://x-minus.pro/ai`, basado en los HAR capturados el 17 y 18/11/2026 y el catálogo de modelos de `ai.v105.min.js` que venía en ellos. El frontend Vue invoca comandos Rust para abrir una sesión, subir audio, consultar estado y guardar MP3; no depende de CORS ni de ejecutar su interfaz web dentro de Tauri.

## Instalación en macOS

Requiere Node.js, Rust/Cargo (`rustup`) y los prerrequisitos de [Tauri 2](https://v2.tauri.app/start/prerequisites/).

```bash
npm install
npm run tauri dev
```

Para compilar la app de escritorio:

```bash
npm run tauri build
```

`npm run dev` sirve solo para visualizar la interfaz, sin el backend HTTP.

## Flujo de uso

1. Pulsa **Inicializar API**. La app intenta recuperar un `auth_key` temporal de la página actual. Si la web requiere tu sesión, puedes introducir **tu propia** cabecera `Cookie` de ese dominio en el campo opcional. No se conserva en disco ni se envía a la CDN.
2. Selecciona el audio (MP3, WAV, FLAC, M4A, OGG).
3. Selecciona el **número de stems**: `2`, `4`, `6` o `Especiales` (1–3 pistas de restauración/coros).
4. Selecciona un **tipo de separación** y un **modelo**. La interfaz muestra exactamente qué stems devolverá. Se permiten varios modelos para la misma separación.
5. Pulsa **Separar**. La app sube el audio y consulta el estado cada cinco segundos (máximo 60 consultas por tanda, reanudables).
6. Cuando X-Minus devuelve `done`, elige la carpeta de destino. Rust descarga cada pista usando el identificador de trabajo original, siguiendo hasta **seis redirecciones seguras** (incluida la etapa `/encode` cuando exista).
7. Si alguna descarga falla, el botón se convierte en **Reintentar stems pendientes** y no repite las que ya se han guardado.

### Recuperar una separación ya hecha (sin volver a subir el MP3)

Tras reiniciar la aplicación, el estado del trabajo antiguo no permanece en memoria. En v0.3 puedes abrir **Recuperar un trabajo ya procesado** en la sección 02, seleccionar el modelo del trabajo original, introducir su `job_id` (visible en la interfaz de v0.2) y un nombre opcional. El programa reconstruye el trabajo en memoria y comprueba el estado si la sesión actual lo permite. Si esa consulta falla porque la sesión ha caducado, aún puedes intentar la descarga; el servidor decide si sigue disponible. **No realiza otra subida ni consume una nueva separación.**

Para una recuperación puntual de batería y bajo desde Terminal, sin reinstalar la aplicación, también puedes usar:

```bash
./scripts/recover_stems.sh TU_JOB_ID "$HOME/Downloads" drums bass
```

El script sigue todos los redirects con `curl -L`, no introduce credenciales y comprueba que se hayan recibido bytes. Si el servidor ha caducado los archivos o requiere otra autenticación, devolverá error sin alterar la sesión. No está probado en directo contra el proveedor desde este entorno.


## Catálogo de modelos

El catálogo de `src/catalog.json` se obtuvo de la estructura `B.separation_type` dentro del JavaScript capturado; **no son nombres de modelos inventados**.

- **69 entradas en 16 tipos de selección**; de ellas, 45 no están marcadas con `is_test` y 24 son modelos internos de prueba.
- En el modo `2 stems → Voz e instrumental` están disponibles en el catálogo publicado los cinco modelos visibles en tus capturas: **Mel-Roformer deux (becruily)**, **BS-Roformer HyperACE (unwa)**, **BS-Roformer Resurrection (unwa)**, **Mel-RoFormer by Gabox Fv7** y **Mel-RoFormer by unwa v1e+**.
- `4 stems` contiene **BS-RoFormer ZFTurbo**, **Demucs v4** y **Demucs Drumsep**. Drumsep entrega **bombo, caja, toms y platos**, no voz/bajo/batería/instrumental.
- `6 stems` utiliza `BS-RoFormer 6-stems SW` (modelo con acceso restringido por el servicio).
- También incluye bajo, batería, guitarra, saxofón/viento, piano, cuerdas, coros, de-eco, de-reverb, de-crowd, restauración y experimentales.

Cada entrada asocia los campos **reales** `separation_type`, `separation`, `model` y una lista de stems. La asociación se valida *también en Rust*, sin fiarse solo de los parámetros enviados por Vue. No se permiten combinaciones de categoría/modelo desconocidas.

**Acceso y limitaciones:** el catálogo JS incluye reglas de acceso `is_premium`, `is_tester`, `is_sng_country` y un nivel de acceso por modelo. El wrapper evalúa la misma condición observada en el JS para habilitar o bloquear modelos. Los marcados con `is_test` solo aparecen al activar explícitamente la casilla de modelos de prueba; **no se garantiza que el servidor los acepte**. La app no intenta esquivar restricciones ni cuotas. Un modelo puede desaparecer sin aviso.

**Importante sobre los stems:** la web no ofrece una cifra arbitraria de pistas. El número de stems es consecuencia del modelo y de la categoría elegidos. Las opciones visuales `2`, `4`, `6`, `Especiales` organizan las combinaciones compatibles.

## Contrato observado (HAR)

| Operación | Ruta / detalles |
|---|---|
| Obtener sesión | `GET https://x-minus.pro/ai`, lee `#vocal-cut-auth-key`, `#vocal-cut-fp`, `aiInitData.version` y capacidades de la sesión |
| Subir archivo | `POST https://mmd.uvronline.vip/upload/vocalCutAi?catch-file`, `multipart/form-data` con `myfile` y parámetros |
| Consultar estado | `POST .../upload/vocalCutAi?check-job-status` con `job_id`, `auth_key`, `locale` |
| Descargar | `GET .../dl/vocalCutAi?job-id=...&stem=...&fmt=mp3&cdn=0` → 302 a CDN (`*.uvronline.app`); algunos stems requieren otra redirección intermedia por `/encode` |

Parámetros de subida: `auth_key`, `locale=es_ES`, `separation`, `separation_type`, `format=mp3`, `version`, `model`, `aggressiveness=2`, `lvpanning=center`, `uvrbve_ct=auto`, `pre_rate=100`, `bve_preproc=auto`, `show_setting_format`, `hostname=x-minus.pro`, `client_fp`, `myfile`.

**Dos separaciones verificadas mediante tráfico real:**

- 17/11: `separation_type=vocals_music`, `separation=inst_vocal`, `model=mel_roformer_deux_becruily`, descargas `vocal`/`inst`.
- 18/11: `separation_type=splitter`, `separation=bdov`, `model=htdemucs`, cuatro stems `vocal`/`inst`/`drums`/`bass`. El HAR prueba la cadena de descarga, incluida la redirección adicional de `/encode` para `drums` y `bass`; las respuestas de contenido del navegador son `206` (rango parcial), no el MP3 completo.

Los trabajos devuelven `accepted`, `job_id` y luego `done`; puede aparecer `similar_job_id`, que permite reutilización en servidor. Los demás modelos y el modo de seis pistas siguen **identificados en JS, pero no verificados con tráfico de separación real**.

`check-hash` es una optimización opcional de la página para ficheros grandes, fuera del flujo mínimo del wrapper.

## Comprobaciones

El script `scripts/verify_har.py` busca el contrato de subida/estado/descarga sin mostrar secretos:

```bash
python3 scripts/verify_har.py '/ruta/a/x-minus.pro(1).har'
python3 scripts/verify_redirects.py '/ruta/a/x-minus.pro(2).har'
```

Rust incluye pruebas unitarias de lectura de sesión, seguridad de redirecciones y asociación modelo/descargas (`cargo test` dentro de `src-tauri`).

### Estado

- La estructura del catálogo y la correspondencia stems/modelo se han comprobado localmente.
- La cadena de **1 redirección** para `vocal`/`inst` y la de **2 redirecciones** para `drums`/`bass` se han comprobado contra el HAR del 18/11 con `verify_redirects.py`.
- Se han añadido pruebas unitarias Rust para resolver redirecciones relativas y rechazar destinos no autorizados.
- **No se ha compilado ni ejecutado la aplicación en esta sesión**: este entorno no dispone de Cargo; `npm install --offline` no ha podido resolver las dependencias sin caché.
- El servicio puede cambiar URLs, parámetros, cuotas y controles de autenticación. Antes de utilizarlo de forma continuada, comprueba condiciones de uso y limitaciones aplicables.

### Seguridad

- No distribuyas los HAR originales: pueden contener credenciales.
- El programa no almacena `auth_key` ni cookies en el código o en disco.
- Las redirecciones de descarga se comprueban por HTTPS y dominio en **cada salto** y se limitan a seis; nunca se envían secretos a los servidores de descarga.
- Una separación ya procesada puede reutilizar un trabajo previo del servidor; no se fuerza un identificador de descarga diferente.
