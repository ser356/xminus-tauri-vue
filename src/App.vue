<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { downloadDir } from '@tauri-apps/api/path';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import catalog from './catalog.json';

// ---------- Tipos que devuelve Rust ----------
type ModelInfo = { id: string; name: string; stems: string[]; access_level: number; is_test: boolean; heavy: boolean };
type PresetInfo = { id: string; label: string; models: ModelInfo[] };
type SessionInfo = { version: string; using_account: boolean; account_expired: boolean; is_premium: boolean; is_tester: boolean; is_sng_country: boolean };
type JobInfo = { job_id: string; stems: string[]; source_filename: string; status: string; requests_left: number | null; reused_previous_result: boolean };
type StatusInfo = { status: string; remaining_time: number | null; error: string | null };
type StemResult = { stem: string; path: string | null; error: string | null };
type Recent = { job_id: string; name: string; preset: string; model: string; label: string; date: number };

// ---------- Qué puede hacer el usuario, en lenguaje sencillo ----------
type Choice = { label: string; preset: string; model?: string };
type Goal = { id: string; icon: string; title: string; text: string; choices: Choice[] };

const goals: Goal[] = [
  { id: 'vocals', icon: '🎤', title: 'Voz y música', text: 'Separa la voz del acompañamiento. Ideal para karaoke o acapellas.',
    choices: [{ label: 'Voz y música', preset: 'vocals_music', model: 'mel_roformer_deux_becruily' }] },
  { id: 'band', icon: '🎛️', title: 'Por instrumentos', text: 'Voz, batería, bajo y el resto, cada uno en su propia pista.',
    choices: [
      { label: '4 pistas', preset: 'splitter_4', model: 'htdemucs' },
      { label: '6 pistas (+ guitarra y piano)', preset: 'splitter_6' },
    ] },
  { id: 'single', icon: '🎸', title: 'Un solo instrumento', text: 'Saca un instrumento concreto y deja el resto de la música aparte.',
    choices: [
      { label: 'Batería', preset: 'drums_other', model: 'htdemucs' },
      { label: 'Bajo', preset: 'bass_other', model: 'htdemucs' },
      { label: 'Guitarra', preset: 'guitar_other', model: 'htdemucs' },
      { label: 'Piano', preset: 'piano_other' },
      { label: 'Saxo y viento', preset: 'wind_other' },
      { label: 'Cuerdas', preset: 'strings_other' },
      { label: 'Piezas de la batería', preset: 'splitter_4', model: 'htdemucs_drumsep' },
    ] },
  { id: 'choirs', icon: '👥', title: 'Voz principal y coros', text: 'Distingue al cantante principal de las voces de fondo.',
    choices: [
      { label: 'Voz principal, coros y música', preset: 'bv_other', model: 'mel_roformer_lv_bv_chain1' },
      { label: 'Karaoke que conserva los coros', preset: 'lv_other' },
    ] },
  { id: 'clean', icon: '✨', title: 'Limpiar el audio', text: 'Quita reverberación, eco, ruido o el sonido del público.',
    choices: [
      { label: 'Quitar reverberación', preset: 'dereverb' },
      { label: 'Quitar eco', preset: 'deecho' },
      { label: 'Quitar ruido', preset: 'denoise' },
      { label: 'Quitar público', preset: 'decrowd' },
    ] },
  { id: 'lab', icon: '🧪', title: 'Experimentos', text: 'Funciones en pruebas: voces de hombre y mujer, o el centro de la mezcla.',
    choices: [
      { label: 'Voz de hombre y de mujer', preset: 'experimental', model: 'bs_roformer_mf' },
      { label: 'Extraer el centro', preset: 'experimental', model: 'similarity_m23c' },
    ] },
];

const stemInfo: Record<string, [string, string]> = {
  vocal: ['🎤', 'Voz'], inst: ['🎶', 'Instrumental'], drums: ['🥁', 'Batería'], bass: ['🎸', 'Bajo'],
  guitar: ['🎸', 'Guitarra'], piano: ['🎹', 'Piano'], toms: ['🥁', 'Toms'], cymbals: ['🥁', 'Platos'],
  snare: ['🥁', 'Caja'], kick: ['🥁', 'Bombo'], wind: ['🎷', 'Viento'], strings: ['🎻', 'Cuerdas'],
  acguitar: ['🪕', 'Guitarra acústica'], lv: ['🎤', 'Voz principal'], bv: ['👥', 'Coros'],
  male: ['🙋‍♂️', 'Voz de hombre'], female: ['🙋‍♀️', 'Voz de mujer'], enhanced: ['✨', 'Audio mejorado'],
  similarity: ['🎯', 'Centro de la mezcla'],
};
function stemIcon(stem: string) { return stemInfo[stem]?.[0] ?? '🎵'; }
function stemName(stem: string, all: string[] = []) {
  // «inst» es el instrumental si hay voz; si no, es «lo demás».
  if (stem === 'inst' && !all.includes('vocal') && all.length > 0) return 'Resto de la música';
  return stemInfo[stem]?.[1] ?? stem;
}

const presets: PresetInfo[] = catalog.modes;
const findPreset = (id: string) => presets.find((p) => p.id === id) ?? null;

// ---------- Estado ----------
type Phase = 'pick' | 'uploading' | 'processing' | 'ready' | 'saving' | 'saved' | 'slow';
const session = ref<SessionInfo | null>(null);
const connecting = ref(false);
const connectError = ref('');
const needsLogin = ref(false);
const audioPath = ref('');
const dragging = ref(false);
const goalId = ref('vocals');
const choiceIndex = ref(0);
const modelOverride = ref('');
const showTestModels = ref(false);
const manualCookie = ref('');
const recoveryCode = ref('');
const phase = ref<Phase>('pick');
const job = ref<JobInfo | null>(null);
const remaining = ref<number | null>(null);
const outputs = ref<StemResult[]>([]);
const error = ref('');
const recents = ref<Recent[]>(loadRecents());
let timer: ReturnType<typeof setTimeout> | undefined;
let generation = 0;
let checks = 0;
let unlistenDrop: (() => void) | undefined;
const unlisteners: UnlistenFn[] = [];

const goal = computed(() => goals.find((g) => g.id === goalId.value)!);
const choice = computed(() => goal.value.choices[choiceIndex.value] ?? goal.value.choices[0]);
const preset = computed(() => findPreset(choice.value.preset));
const availableModels = computed(() => (preset.value?.models ?? [])
  .filter((m) => !m.is_test || showTestModels.value || m.id === choice.value.model));

function isAllowed(m: ModelInfo | null | undefined): boolean {
  if (!m) return false;
  const s = session.value;
  if (!s) return true;
  return m.access_level === 0 || m.access_level === 3 || s.is_premium || s.is_tester
    || (m.access_level === 1 && !s.is_sng_country);
}
function defaultModel(c: Choice): ModelInfo | null {
  const list = findPreset(c.preset)?.models ?? [];
  return list.find((m) => m.id === c.model)
    ?? list.find((m) => !m.is_test && m.access_level === 0)
    ?? list.find((m) => !m.is_test && isAllowed(m))
    ?? list.find((m) => !m.is_test) ?? list[0] ?? null;
}
const model = computed(() => availableModels.value.find((m) => m.id === modelOverride.value) ?? defaultModel(choice.value));
const stems = computed(() => model.value?.stems ?? []);
const needsPremium = computed(() => !!model.value && !isAllowed(model.value));
const choiceNeedsPremium = (c: Choice) => !isAllowed(defaultModel(c));

const fileName = computed(() => audioPath.value.split(/[\\/]/).pop() ?? '');
const canStart = computed(() => !!session.value && !!audioPath.value && !!model.value && !needsPremium.value && phase.value === 'pick');
const pending = computed(() => job.value?.stems.filter((s) => !outputs.value.some((o) => o.stem === s && o.path)) ?? []);
const failedSome = computed(() => outputs.value.some((o) => !o.path));

const accountLabel = computed(() => {
  if (connecting.value) return 'Conectando…';
  if (!session.value) return 'Sin conexión';
  if (session.value.is_premium) return 'Cuenta Premium';
  return session.value.using_account ? 'Tu cuenta' : 'Modo gratuito';
});
const remainingText = computed(() => {
  const s = remaining.value;
  if (s == null || s <= 0) return 'Esto suele tardar uno o dos minutos.';
  if (s < 60) return 'Ya casi está, quedan unos segundos.';
  return `Quedan unos ${Math.ceil(s / 60)} minutos.`;
});

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// ---------- Conexión automática ----------
async function connect(cookie?: string) {
  connecting.value = true;
  connectError.value = '';
  try {
    session.value = await invoke<SessionInfo>('initialize_session', { sessionCookie: cookie || null });
    needsLogin.value = false;
    if (cookie) manualCookie.value = '';
  } catch (e) {
    const message = errorText(e);
    // Sin sesión válida el servicio manda a su página de login: se pide entrar.
    if (message.includes('LOGIN_REQUIRED')) { needsLogin.value = true; session.value = null; }
    else connectError.value = message;
  } finally {
    connecting.value = false;
  }
}

// ---------- Iniciar sesión (ventana de X-Minus, vale también «Entrar con Google») ----------
const loggingIn = ref(false);
const loginLoading = ref(false);
async function openLogin() {
  try { loginLoading.value = true; await invoke('open_login'); loggingIn.value = true; }
  catch (e) { loginLoading.value = false; connectError.value = errorText(e); }
}
async function logout() {
  await invoke('logout').catch(() => {});
  loggingIn.value = false;
  await connect();
}

// ---------- Elegir canción ----------
const audioExt = ['mp3', 'wav', 'flac', 'm4a', 'ogg'];
async function pickFile() {
  if (phase.value !== 'pick') return;
  const result = await open({ multiple: false, directory: false, filters: [{ name: 'Canciones', extensions: audioExt }] });
  if (typeof result === 'string') { audioPath.value = result; error.value = ''; }
}

function selectGoal(id: string) {
  if (phase.value !== 'pick') return;
  goalId.value = id;
  choiceIndex.value = 0;
  modelOverride.value = '';
}
function selectChoice(i: number) {
  choiceIndex.value = i;
  modelOverride.value = '';
}

// ---------- Seguimiento del procesamiento ----------
function stopPolling() {
  generation += 1;
  if (timer) clearTimeout(timer);
  timer = undefined;
}

function startPolling() {
  stopPolling();
  checks = 0;
  phase.value = 'processing';
  void poll(generation);
}

async function poll(token: number) {
  if (token !== generation) return;
  try {
    const st = await invoke<StatusInfo>('check_status');
    if (token !== generation) return;
    remaining.value = st.remaining_time;
    if (st.status === 'done') { phase.value = 'ready'; void save(); return; }
    if (['failed', 'error', 'rejected'].includes(st.status)) {
      fail(st.error || 'No se ha podido separar esta canción. Prueba con otra opción o con otro archivo.');
      return;
    }
  } catch (e) {
    if (token !== generation) return;
    // Una separación recuperada puede descargarse aunque ya no se pueda consultar.
    if (job.value?.status === 'restored') { phase.value = 'ready'; void save(); return; }
    if (checks > 3) { fail(errorText(e)); return; }
  }
  checks += 1;
  if (checks >= 240) { phase.value = 'slow'; return; }
  // Cada 5 s los primeros 5 minutos; después, cada 15 s para no saturar al servicio.
  timer = setTimeout(() => void poll(token), checks < 60 ? 5000 : 15000);
}

function fail(message: string) {
  stopPolling();
  error.value = message;
  phase.value = 'pick';
}

// ---------- Separar ----------
async function start() {
  if (!canStart.value || !model.value || !preset.value) return;
  error.value = '';
  outputs.value = [];
  remaining.value = null;
  phase.value = 'uploading';
  try {
    job.value = await invoke<JobInfo>('start_separation', {
      filePath: audioPath.value, presetId: preset.value.id, modelId: model.value.id,
    });
    remember(job.value, preset.value.id, model.value.id, choice.value.label);
    startPolling();
  } catch (e) {
    job.value = null;
    fail(errorText(e));
  }
}

// ---------- Guardar ----------
async function save() {
  if (!job.value || pending.value.length === 0) return;
  let dir: string | null = null;
  try {
    const picked = await open({ directory: true, multiple: false, title: '¿Dónde guardo las pistas?', defaultPath: await downloadDir() });
    dir = typeof picked === 'string' ? picked : null;
  } catch { /* se trata como cancelado */ }
  if (!dir) { phase.value = 'ready'; return; }
  phase.value = 'saving';
  error.value = '';
  try {
    const fresh = await invoke<StemResult[]>('download_stems', { directory: dir, requestedStems: pending.value });
    const previous = outputs.value;
    outputs.value = job.value.stems.map((stem) => fresh.find((r) => r.stem === stem)
      ?? previous.find((r) => r.stem === stem) ?? { stem, path: null, error: 'No descargada' });
    phase.value = pending.value.length === 0 ? 'saved' : 'ready';
    if (failedSome.value) error.value = 'Algunas pistas no se han podido guardar. Puedes volver a intentarlo.';
  } catch (e) {
    phase.value = 'ready';
    error.value = errorText(e);
  }
}

async function reveal(path: string) {
  try { await invoke('reveal_file', { path }); } catch (e) { error.value = errorText(e); }
}

function reset() {
  stopPolling();
  job.value = null;
  outputs.value = [];
  error.value = '';
  audioPath.value = '';
  phase.value = 'pick';
}

// ---------- Canciones recientes (para volver a descargar sin subir de nuevo) ----------
function loadRecents(): Recent[] {
  try { return JSON.parse(localStorage.getItem('recent-jobs') ?? '[]'); } catch { return []; }
}
function remember(j: JobInfo, presetId: string, modelId: string, label: string) {
  const name = fileName.value.replace(/\.[^.]+$/, '') || j.source_filename;
  const entry: Recent = { job_id: j.job_id, name, preset: presetId, model: modelId, label, date: Date.now() };
  recents.value = [entry, ...recents.value.filter((r) => r.job_id !== j.job_id)].slice(0, 8);
  localStorage.setItem('recent-jobs', JSON.stringify(recents.value));
}
function forget(id: string) {
  recents.value = recents.value.filter((r) => r.job_id !== id);
  localStorage.setItem('recent-jobs', JSON.stringify(recents.value));
}
async function restore(jobId: string, name: string, presetId: string, modelId: string) {
  if (!session.value || phase.value !== 'pick') return;
  error.value = '';
  outputs.value = [];
  phase.value = 'uploading';
  try {
    job.value = await invoke<JobInfo>('restore_job', { jobId, sourceFilename: name || 'cancion', presetId, modelId });
    startPolling();
  } catch (e) {
    job.value = null;
    fail(errorText(e));
  }
}
function restoreByCode() {
  if (!model.value || !preset.value) return;
  void restore(recoveryCode.value.trim(), fileName.value.replace(/\.[^.]+$/, ''), preset.value.id, model.value.id);
}
function ago(ts: number) {
  const min = Math.round((Date.now() - ts) / 60000);
  if (min < 60) return min <= 1 ? 'hace un momento' : `hace ${min} min`;
  const h = Math.round(min / 60);
  if (h < 24) return `hace ${h} h`;
  const d = Math.round(h / 24);
  return d === 1 ? 'ayer' : `hace ${d} días`;
}

// ---------- Arrastrar y soltar ----------
onMounted(async () => {
  void connect();
  // La ventana de login avisa sola: carga de página, login completado o cierre.
  try {
    unlisteners.push(
      await listen<boolean>('login-loading', (e) => { loginLoading.value = e.payload; }),
      await listen('login-detected', async () => { loggingIn.value = false; loginLoading.value = false; await connect(); }),
      await listen('login-closed', () => { loggingIn.value = false; loginLoading.value = false; }),
    );
  } catch { /* fuera de Tauri */ }
  try {
    unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === 'over') { dragging.value = phase.value === 'pick'; return; }
      if (p.type === 'leave') { dragging.value = false; return; }
      if (p.type === 'drop') {
        dragging.value = false;
        if (phase.value !== 'pick') return;
        const file = p.paths.find((f) => audioExt.includes(f.split('.').pop()?.toLowerCase() ?? ''));
        if (file) { audioPath.value = file; error.value = ''; }
        else error.value = 'Ese archivo no parece una canción. Usa MP3, WAV, FLAC, M4A u OGG.';
      }
    });
  } catch { /* fuera de Tauri (vista previa en navegador) */ }
});
onBeforeUnmount(() => { stopPolling(); unlistenDrop?.(); unlisteners.forEach((u) => u()); });
</script>

<template>
  <main class="app" :class="{ dragging }">
    <header class="header">
      <div class="logo" aria-hidden="true"><span /><span /><span /><span /></div>
      <div class="brand">
        <h1>Separador de pistas</h1>
        <p>Divide cualquier canción en voz, batería, bajo y más</p>
      </div>
      <span class="account" :class="{ ok: session, premium: session?.is_premium }">
        <i class="dot" />{{ accountLabel }}
      </span>
      <button v-if="session?.using_account" class="ghost" @click="logout">Cerrar sesión</button>
      <button v-else-if="session || needsLogin" class="ghost" :disabled="loggingIn" @click="openLogin">Iniciar sesión</button>
    </header>

    <div v-if="connectError" class="banner warn">
      <span>No se ha podido conectar con el servicio. {{ connectError }}</span>
      <button class="link" :disabled="connecting" @click="connect()">Reintentar</button>
    </div>

    <div v-if="needsLogin && !loggingIn" class="banner info">
      <span>Para empezar, inicia sesión con tu cuenta de X-Minus (puedes usar Google).</span>
      <button class="ghost" @click="openLogin">Iniciar sesión</button>
    </div>

    <div v-if="loggingIn" class="banner info">
      <span>{{ loginLoading ? 'Cargando la página de inicio de sesión…' : 'Inicia sesión en la ventana que se ha abierto (puedes usar Google). Se detectará automáticamente.' }}</span>
      <span v-if="loginLoading" class="topbar" aria-hidden="true"><i /></span>
    </div>

    <div v-if="session?.account_expired" class="banner info">
      <span>Tu sesión de X-Minus guardada en la app ha caducado, así que estás en modo gratuito. Pulsa «Iniciar sesión» para volver a entrar con tu cuenta.</span>
    </div>

    <!-- ========== Paso 1 y 2: elegir canción y qué sacar ========== -->
    <template v-if="phase === 'pick'">
      <section class="card">
        <h2><span class="num">1</span>Elige una canción</h2>
        <button class="drop" :class="{ has: audioPath }" @click="pickFile">
          <template v-if="audioPath">
            <span class="drop-icon">🎵</span>
            <span class="drop-main">{{ fileName }}</span>
            <span class="drop-sub">Pulsa para cambiarla</span>
          </template>
          <template v-else>
            <span class="drop-icon">📂</span>
            <span class="drop-main">Arrastra aquí tu canción o pulsa para buscarla</span>
            <span class="drop-sub">MP3, WAV, FLAC, M4A u OGG</span>
          </template>
        </button>
      </section>

      <section class="card">
        <h2><span class="num">2</span>¿Qué quieres separar?</h2>
        <div class="goals">
          <button v-for="g in goals" :key="g.id" class="goal" :class="{ active: goalId === g.id }" @click="selectGoal(g.id)">
            <span class="goal-icon">{{ g.icon }}</span>
            <span class="goal-title">{{ g.title }}</span>
            <span class="goal-text">{{ g.text }}</span>
          </button>
        </div>

        <div v-if="goal.choices.length > 1" class="chips">
          <button v-for="(c, i) in goal.choices" :key="c.label" class="chip" :class="{ active: choiceIndex === i }" @click="selectChoice(i)">
            {{ c.label }}<span v-if="choiceNeedsPremium(c)" class="tag">Premium</span>
          </button>
        </div>

        <div class="preview">
          <span class="preview-title">Vas a obtener {{ stems.length === 1 ? '1 pista' : `${stems.length} pistas` }}:</span>
          <div class="tracks">
            <span v-for="s in stems" :key="s" class="track"><span>{{ stemIcon(s) }}</span>{{ stemName(s, stems) }}</span>
          </div>
        </div>

        <p v-if="needsPremium" class="banner info">
          Esta opción solo está disponible con una cuenta Premium de X-Minus. Elige otra opción o abre «Ajustes avanzados» para usar tu cuenta.
        </p>
        <p v-else-if="model?.heavy" class="note">⏳ Esta opción da muy buen resultado pero tarda algo más.</p>
        <p v-else-if="model?.is_test" class="note">🧪 Opción experimental: puede que no funcione siempre.</p>
      </section>

      <p v-if="error" class="banner error" role="alert">{{ error }}</p>

      <button class="cta" :disabled="!canStart" @click="start">
        {{ !session ? 'Conectando…' : !audioPath ? 'Elige primero una canción' : needsPremium ? 'Necesita Premium' : 'Separar canción' }}
      </button>

      <!-- Canciones ya separadas -->
      <section v-if="recents.length" class="card soft">
        <h3>Separadas recientemente</h3>
        <p class="muted small">Vuelve a guardar sus pistas sin tener que subir la canción otra vez.</p>
        <ul class="recents">
          <li v-for="r in recents" :key="r.job_id">
            <div>
              <strong>{{ r.name }}</strong>
              <span class="muted small">{{ r.label }} · {{ ago(r.date) }}</span>
            </div>
            <button class="ghost" :disabled="!session" @click="restore(r.job_id, r.name, r.preset, r.model)">Descargar</button>
            <button class="icon-btn" title="Quitar de la lista" @click="forget(r.job_id)">✕</button>
          </li>
        </ul>
      </section>

      <details class="advanced">
        <summary>Ajustes avanzados</summary>
        <div class="adv-grid">
          <label class="field">
            <span>Modelo de inteligencia artificial</span>
            <select :value="model?.id" @change="modelOverride = ($event.target as HTMLSelectElement).value">
              <option v-for="m in availableModels" :key="m.id" :value="m.id">
                {{ m.name }}{{ !isAllowed(m) ? ' · Premium' : '' }}{{ m.is_test ? ' · beta' : '' }}{{ m.heavy ? ' · lento' : '' }}
              </option>
            </select>
            <label class="check"><input v-model="showTestModels" type="checkbox" /> Mostrar también modelos en pruebas</label>
          </label>

          <label class="field">
            <span>Usar mi cuenta de X-Minus</span>
            <div class="row">
              <input v-model="manualCookie" type="password" autocomplete="off" placeholder="Pega aquí la cookie de tu navegador" />
              <button class="ghost" :disabled="connecting || !manualCookie.trim()" @click="connect(manualCookie.trim())">Conectar</button>
            </div>
            <small class="muted">Solo hace falta si no se incluyó al compilar la app. No se guarda.</small>
          </label>

          <label class="field">
            <span>Recuperar con un código de separación</span>
            <div class="row">
              <input v-model="recoveryCode" type="text" spellcheck="false" autocomplete="off" placeholder="Código de 40 caracteres o más" />
              <button class="ghost" :disabled="!session || !recoveryCode.trim() || needsPremium" @click="restoreByCode">Recuperar</button>
            </div>
            <small class="muted">Elige arriba el mismo tipo de separación que usaste aquella vez.</small>
          </label>
        </div>
      </details>
    </template>

    <!-- ========== Procesando ========== -->
    <section v-else-if="phase === 'uploading' || phase === 'processing' || phase === 'slow'" class="card status">
      <div class="spinner" :class="{ paused: phase === 'slow' }" aria-hidden="true"><span /><span /><span /><span /><span /></div>
      <template v-if="phase === 'uploading'">
        <h2>Subiendo tu canción…</h2>
        <p class="muted">Según el tamaño del archivo puede tardar un poco.</p>
      </template>
      <template v-else-if="phase === 'processing'">
        <h2>Separando las pistas…</h2>
        <p class="muted">{{ remainingText }} Puedes dejar la app abierta mientras tanto.</p>
        <div class="bar"><span /></div>
      </template>
      <template v-else>
        <h2>Está tardando más de lo normal</h2>
        <p class="muted">El servicio puede estar muy ocupado.</p>
        <div class="row center">
          <button class="cta small" @click="startPolling">Seguir esperando</button>
          <button class="ghost" @click="reset">Cancelar</button>
        </div>
      </template>
      <p v-if="job" class="muted small song">🎵 {{ job.source_filename }}</p>
    </section>

    <!-- ========== Resultado ========== -->
    <section v-else class="card status">
      <div class="done-icon">{{ phase === 'saved' ? '🎉' : '✅' }}</div>
      <h2 v-if="phase === 'saved'">¡Listo! Tus pistas están guardadas</h2>
      <h2 v-else-if="phase === 'saving'">Guardando las pistas…</h2>
      <h2 v-else>Tu canción ya está separada</h2>
      <p v-if="job" class="muted song">🎵 {{ job.source_filename }}</p>

      <ul v-if="job" class="results">
        <li v-for="s in job.stems" :key="s">
          <span class="r-icon">{{ stemIcon(s) }}</span>
          <span class="r-name">{{ stemName(s, job.stems) }}</span>
          <template v-for="o in outputs.filter((x) => x.stem === s)" :key="o.stem">
            <button v-if="o.path" class="ghost" @click="reveal(o.path)">Mostrar</button>
            <span v-else class="r-error">No se pudo guardar</span>
          </template>
          <span v-if="phase === 'saving' && pending.includes(s)" class="muted small">guardando…</span>
        </li>
      </ul>

      <p v-if="error" class="banner error" role="alert">{{ error }}</p>
      <p v-if="job?.requests_left != null && phase === 'saved'" class="muted small">
        Te quedan {{ job.requests_left }} separaciones disponibles hoy.
      </p>

      <div class="row center">
        <button v-if="phase === 'ready'" class="cta small" @click="save">
          {{ failedSome ? 'Reintentar las que faltan' : 'Guardar pistas' }}
        </button>
        <button class="ghost" :disabled="phase === 'saving'" @click="reset">Separar otra canción</button>
      </div>
      <details v-if="job" class="code">
        <summary>Código de esta separación</summary>
        <code>{{ job.job_id }}</code>
      </details>
    </section>

    <div v-if="dragging" class="drop-overlay"><span>Suelta la canción aquí 🎵</span></div>

    <footer>Funciona con el servicio de X-Minus. App no oficial.</footer>
  </main>
</template>
