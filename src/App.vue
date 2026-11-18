<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { downloadDir } from '@tauri-apps/api/path';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import studioRabbit from './assets/studio.webp';
import portraitRabbit from './assets/portrait.webp';
import backRabbit from './assets/back.webp';
import frontRabbit from './assets/front.webp';
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
  { id: 'vocals', icon: 'M9 5a3 3 0 0 1 6 0v7a3 3 0 0 1-6 0z M5 11v1a7 7 0 0 0 14 0v-1 M12 19v3m-4 0h8', title: 'Voz y música', text: 'Separa la voz del acompañamiento. Ideal para karaoke o acapellas.',
    choices: [{ label: 'Voz y música', preset: 'vocals_music', model: 'mel_roformer_deux_becruily' }] },
  { id: 'band', icon: 'M5 3v6m0 4v8m7-18v11m0 4v3m7-18v3m0 4v11 M2 9h6v4H2z M9 14h6v4H9z M16 6h6v4h-6z', title: 'Por instrumentos', text: 'Voz, batería, bajo y el resto, cada uno en su propia pista.',
    choices: [
      { label: '4 pistas', preset: 'splitter_4', model: 'htdemucs' },
      { label: '6 pistas (+ guitarra y piano)', preset: 'splitter_6' },
    ] },
  { id: 'single', icon: 'M9 17V6l11-3v11 M9 9l11-3 M9 17a3 2.5 0 1 1-6 0 3 2.5 0 1 1 6 0 M20 14a3 2.5 0 1 1-6 0 3 2.5 0 1 1 6 0', title: 'Un solo instrumento', text: 'Saca un instrumento concreto y deja el resto de la música aparte.',
    choices: [
      { label: 'Batería', preset: 'drums_other', model: 'htdemucs' },
      { label: 'Bajo', preset: 'bass_other', model: 'htdemucs' },
      { label: 'Guitarra', preset: 'guitar_other', model: 'htdemucs' },
      { label: 'Piano', preset: 'piano_other' },
      { label: 'Saxo y viento', preset: 'wind_other' },
      { label: 'Cuerdas', preset: 'strings_other' },
      { label: 'Piezas de la batería', preset: 'splitter_4', model: 'htdemucs_drumsep' },
    ] },
  { id: 'choirs', icon: 'M2 19v-2a5 5 0 0 1 10 0v2 M10 7a3 3 0 1 1-6 0 3 3 0 1 1 6 0 M15 5a3 3 0 1 1 0 6m0 2a6 6 0 0 1 7 6', title: 'Voz principal y coros', text: 'Distingue al cantante principal de las voces de fondo.',
    choices: [
      { label: 'Voz principal, coros y música', preset: 'bv_other', model: 'mel_roformer_lv_bv_chain1' },
      { label: 'Karaoke que conserva los coros', preset: 'lv_other' },
    ] },
  { id: 'clean', icon: 'M12 2l2.6 7.4L22 12l-7.4 2.6L12 22l-2.6-7.4L2 12l7.4-2.6z', title: 'Limpiar el audio', text: 'Quita reverberación, eco, ruido o el sonido del público.',
    choices: [
      { label: 'Quitar reverberación', preset: 'dereverb' },
      { label: 'Quitar eco', preset: 'deecho' },
      { label: 'Quitar ruido', preset: 'denoise' },
      { label: 'Quitar público', preset: 'decrowd' },
    ] },
  { id: 'lab', icon: 'M9 3h6m-5 0v7L4 20q-.5 1 1 1h14q1.5 0 1-1l-6-10V3 M7 16h10', title: 'Experimentos', text: 'Funciones en pruebas: voces de hombre y mujer, o el centro de la mezcla.',
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
  if (!s || s.using_account) return true; // Con cuenta iniciada decide el servidor.
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

const rabbitPose = computed(() => {
  if (phase.value === 'pick') return audioPath.value ? portraitRabbit : studioRabbit;
  return ['uploading', 'processing', 'slow'].includes(phase.value) ? backRabbit : frontRabbit;
});

const rabbitPhotos = [backRabbit, frontRabbit, portraitRabbit, studioRabbit];
const photoStrip = computed(() => rabbitPhotos
  .filter(photo => photo !== rabbitPose.value));
const motionPaused = ref(false);

const showRabbitRain = ref(false);
const fallingCutouts = Array.from({ length: 24 }, (_, index) => ({
  photo: rabbitPhotos[index % rabbitPhotos.length],
  style: {
    left: `${(index * 37 + 2) % 98}%`,
    width: `${32 + (index * 13) % 35}px`,
    animationDelay: `${index * 65}ms`,
    animationDuration: `${2400 + (index * 173) % 760}ms`,
    '--drift': `${(index * 31) % 99 - 49}px`,
    '--turn': `${(index * 57) % 270 - 135}deg`,
  },
}));

function toggleMotion() {
  motionPaused.value = !motionPaused.value;
  if (motionPaused.value) showRabbitRain.value = false;
}

function celebrateReady() {
  showRabbitRain.value = !motionPaused.value && !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

const accountLabel = computed(() => {
  if (connecting.value) return 'Conectando…';
  if (!session.value) return 'Sin conexión';
  if (session.value.is_premium) return 'Cuenta Premium';
  return session.value.using_account ? 'Cuenta conectada' : 'Modo gratuito';
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
  } catch (e) {
    const message = errorText(e);
    // Sin sesión válida el servicio manda a su página de login: se pide entrar.
    session.value = null;
    needsLogin.value = message === 'LOGIN_REQUIRED';
    if (!needsLogin.value) connectError.value = message;
  } finally {
    connecting.value = false;
  }
}

// ---------- Iniciar sesión (ventana de X-Minus, vale también «Entrar con Google») ----------
const loggingIn = ref(false);
const loginLoading = ref(false);
async function openLogin() {
  if (loggingIn.value || connecting.value) return;
  loggingIn.value = true;
  loginLoading.value = true;
  connectError.value = '';
  try { await invoke('open_login'); }
  catch (e) { loggingIn.value = false; loginLoading.value = false; connectError.value = errorText(e); }
}
async function cancelLogin() {
  try {
    await invoke('close_login');
    loggingIn.value = false;
    loginLoading.value = false;
  } catch (e) { connectError.value = errorText(e); }
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
    if (st.status === 'done') {
      phase.value = 'ready';
      celebrateReady();
      void save();
      return;
    }
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
  showRabbitRain.value = false;
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
  // La ventana de login avisa sola: carga de página, login completado o cierre.
  try {
    unlisteners.push(
      await listen<boolean>('login-loading', (e) => { loginLoading.value = e.payload; }),
      await listen('login-detected', async () => { loggingIn.value = false; loginLoading.value = false; await connect(); }),
      await listen('login-closed', () => { loggingIn.value = false; loginLoading.value = false; }),
    );
  } catch { /* fuera de Tauri */ }
  void connect();
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
  <main class="app" :class="{ dragging, still: motionPaused }">
    <div class="dedication" aria-hidden="true">
      <svg width="100%" height="100%" focusable="false">
        <defs>
          <pattern id="dedication-print" width="200" height="90" patternUnits="userSpaceOnUse">
            <g fill="currentColor" font-family="Georgia, serif" font-size="23" font-weight="bold" font-style="italic">
              <text id="dedication-line" x="8" y="30" transform="rotate(-4 8 30)">noelia te amo</text>
              <g transform="translate(100 45)">
                <use href="#dedication-line" x="-200"/>
                <use href="#dedication-line"/>
              </g>
            </g>
          </pattern>
        </defs>
        <rect width="100%" height="100%" fill="url(#dedication-print)"/>
      </svg>
    </div>
    <header class="header">
      <img class="logo" src="../src-tauri/icons/128x128.png" width="38" height="38" alt="" />
      <div class="brand">cinna <span>tube</span></div>
      <button class="motion-switch" type="button" aria-label="Pausar animaciones" :aria-pressed="motionPaused"
        :title="motionPaused ? 'Reanudar animaciones' : 'Pausar animaciones'" @click="toggleMotion">
        <svg aria-hidden="true" viewBox="0 0 18 18" fill="currentColor">
          <path v-if="motionPaused" d="M5 3l10 6-10 6z"/>
          <path v-else d="M4 3h4v12H4zm7 0h4v12h-4z"/>
        </svg>
      </button>
      <span class="account" :class="{ ok: session, premium: session?.is_premium }">
        <i class="dot" />{{ accountLabel }}
      </span>
      <button v-if="session?.using_account" class="ghost" @click="logout">Cerrar sesión</button>
      <button v-else class="ghost" :disabled="connecting || loggingIn" @click="openLogin">Iniciar sesión</button>
    </header>

    <section class="sleeve">
      <div class="sleeve-copy">
        <h1><span>Separador</span><br><em>de pistas</em></h1>
        <p>Divide tu canción en voz, instrumentos y pistas independientes.</p>
      </div>
      <div class="art" aria-hidden="true">
        <svg class="sky" viewBox="0 0 420 220" preserveAspectRatio="xMidYMax slice" focusable="false">
          <defs>
            <pattern id="print-dots" width="5" height="5" patternUnits="userSpaceOnUse">
              <circle cx="2" cy="2" r=".95" fill="#28453b"/>
            </pattern>
          </defs>
          <g class="clouds" fill="#fffdf3" stroke="#332d41" stroke-width="1.4">
            <path d="M19 60q-2-13 10-15 4-21 24-14 10 3 12 15 14-2 17 14z"/>
            <path d="M321 81q-1-10 9-13 5-17 20-12 8 2 10 12 12-2 15 13z"/>
          </g>
          <g class="sun"><circle cx="293" cy="42" r="23" fill="#ffcb53"/>
            <path d="M293 8V3m0 78v-5m34-34h6m-80 0h6m58-23 4-4m-55 55 4-4m46 0 4 4m-54-55 4 4" fill="none" stroke="#efa035" stroke-width="4" stroke-linecap="round"/></g>
          <path d="M-20 206Q74 116 172 166T440 127v112H-20z" fill="#8acfa1"/>
          <path d="M-20 204q121-54 237-6t223-14v55H-20z" fill="#5ab793"/>
          <path d="M-20 204q121-54 237-6t223-14v55H-20z" fill="url(#print-dots)" opacity=".55"/>
          <g class="melody" fill="#5b4594"><path d="m80 96 15-4v21a5 4 0 1 1-3-4V97l-9 2v17a5 4 0 1 1-3-4z"/></g>
          <g fill="#fff0a4"><circle cx="69" cy="201" r="3"/><circle cx="328" cy="186" r="3"/><circle cx="339" cy="182" r="2"/></g>
        </svg>
        <img :key="rabbitPose" :src="rabbitPose" alt="" />
      </div>
      <div class="contact-sheet" aria-hidden="true">
        <span v-for="photo in photoStrip" :key="photo"><img :src="photo" alt="" /></span>
      </div>
    </section>

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
      <button class="ghost" @click="cancelLogin">Cerrar ventana</button>
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
          <svg class="upload-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M12 16V3m-5 5 5-5 5 5M4 15v5h16v-5" /></svg>
          <span class="drop-copy"><strong class="drop-main">{{ audioPath ? fileName : 'Arrastra aquí tu canción' }}</strong>
            <span class="drop-sub">MP3, WAV, FLAC, M4A u OGG</span></span>
          <span class="browse">{{ audioPath ? 'Cambiar archivo' : 'Elegir archivo' }}</span>
        </button>
      </section>

      <section class="card">
        <h2><span class="num">2</span>¿Qué quieres separar?</h2>
        <div class="goals">
          <button v-for="g in goals" :key="g.id" class="goal" :aria-pressed="goalId === g.id" :class="{ active: goalId === g.id }" @click="selectGoal(g.id)">
            <svg class="goal-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path :d="g.icon" /></svg>
            <span class="goal-title">{{ g.title }}</span>
            <span class="goal-text">{{ g.text }}</span>
          </button>
        </div>

        <div v-if="goal.choices.length > 1" class="chips">
          <button v-for="(c, i) in goal.choices" :key="c.label" class="chip" :aria-pressed="choiceIndex === i" :class="{ active: choiceIndex === i }" @click="selectChoice(i)">
            {{ c.label }}<span v-if="choiceNeedsPremium(c)" class="tag">Premium</span>
          </button>
        </div>

        <div class="preview">
          <span class="preview-title">Salida · {{ stems.length === 1 ? '1 pista' : `${stems.length} pistas` }}</span>
          <div :key="[goalId, choiceIndex, model?.id].join('-')" class="tracks">
            <span v-for="s in stems" :key="s" class="track"><i class="track-dot" aria-hidden="true" />{{ stemName(s, stems) }}</span>
          </div>
        </div>

        <p v-if="needsPremium" class="banner info">
          Esta opción es de pago. Si tienes cuenta Premium, pulsa «Iniciar sesión» arriba a la derecha.
        </p>
        <p v-else-if="model?.heavy" class="note">Esta opción da muy buen resultado pero tarda algo más.</p>
        <p v-else-if="model?.is_test" class="note">Opción experimental: puede que no funcione siempre.</p>
      </section>

      <p v-if="error" class="banner error" role="alert">{{ error }}</p>

      <button class="cta" :disabled="!canStart" @click="start">
        {{ connecting ? 'Conectando…' : !session ? 'Conexión necesaria' : !audioPath ? 'Elige primero una canción' : needsPremium ? 'Necesita Premium' : 'Separar canción' }}
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
      <p v-if="job" class="muted small song">{{ job.source_filename }}</p>
    </section>

    <!-- ========== Resultado ========== -->
    <section v-else class="card status">

      <h2 v-if="phase === 'saved'">¡Listo! Tus pistas están guardadas</h2>
      <h2 v-else-if="phase === 'saving'">Guardando las pistas…</h2>
      <h2 v-else>Tu canción ya está separada</h2>
      <p v-if="job" class="muted song">{{ job.source_filename }}</p>

      <ul v-if="job" class="results">
        <li v-for="s in job.stems" :key="s">
          <i class="track-dot" aria-hidden="true" />
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

    <div v-if="showRabbitRain" class="rabbit-rain" aria-hidden="true">
      <img v-for="(piece, index) in fallingCutouts" :key="index" :src="piece.photo" :style="piece.style" alt="" />
    </div>

    <div v-if="dragging" class="drop-overlay"><span>Suelta la canción aquí</span></div>

    <footer>cinna 2026</footer>
  </main>
</template>
