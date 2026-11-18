<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import catalog from './catalog.json';

type StemGroup = '2' | '4' | '6' | 'special';
type ModelInfo = {
  id: string;
  name: string;
  backend: string;
  stems: string[];
  access_level: number;
  is_test: boolean;
  heavy: boolean;
  options: string[];
  description_id: string | null;
};
type PresetInfo = {
  id: string;
  separation_type: string;
  label: string;
  description: string;
  group: string;
  models: ModelInfo[];
};
type SessionInfo = {
  version: string;
  is_premium: boolean;
  is_tester: boolean;
  is_sng_country: boolean;
};
type JobInfo = {
  job_id: string;
  stems: string[];
  source_filename: string;
  status: string;
  remaining_time: number | null;
  requests_left: number | null;
  reused_previous_result: boolean;
};
type StatusInfo = { status: string; remaining_time: number | null; error: string | null };
type StemResult = { stem: string; path: string | null; error: string | null };

const presets: PresetInfo[] = catalog.modes;
const options: { id: StemGroup; label: string; subtitle: string }[] = [
  { id: '2', label: '2 stems', subtitle: 'Voz y música, instrumentos y otros' },
  { id: '4', label: '4 stems', subtitle: 'Multipista o batería detallada' },
  { id: '6', label: '6 stems', subtitle: 'Voz, instrumental, batería, bajo, guitarra, piano' },
  { id: 'special', label: 'Especiales', subtitle: 'Coros, restauración y otras pistas' },
];
const stemLabels: Record<string, string> = {
  vocal: 'Voces', inst: 'Instrumental', drums: 'Batería', bass: 'Bajo',
  guitar: 'Guitarra', piano: 'Piano', toms: 'Toms', cymbals: 'Platos',
  snare: 'Caja', kick: 'Bombo', wind: 'Viento', strings: 'Cuerdas',
  acguitar: 'Guitarra acústica', lv: 'Voz principal', bv: 'Coros',
  male: 'Voz masculina', female: 'Voz femenina', enhanced: 'Audio mejorado',
  similarity: 'Centro',
};

const sessionCookie = ref('');
const session = ref<SessionInfo | null>(null);
const audioPath = ref('');
const recoveryId = ref('');
const recoveryName = ref('');
const group = ref<StemGroup>('2');
const presetId = ref('vocals_music');
const modelId = ref('mel_roformer_deux_becruily');
const showTestModels = ref(false);
const job = ref<JobInfo | null>(null);
const status = ref<StatusInfo | null>(null);
const outputs = ref<StemResult[]>([]);
const busy = ref(false);
const downloading = ref(false);
const polling = ref(false);
const error = ref('');
const info = ref('');
let timer: ReturnType<typeof setTimeout> | undefined;
let generation = 0;
let checks = 0;

const locked = computed(() => busy.value || polling.value || downloading.value);
const groupedPresets = computed(() => presets.filter((item) => item.group === group.value));
const preset = computed(() => presets.find((item) => item.id === presetId.value) ?? null);
const models = computed(() => preset.value?.models.filter((item) => showTestModels.value || !item.is_test) ?? []);
const model = computed(() => models.value.find((item) => item.id === modelId.value) ?? null);
const stems = computed(() => model.value?.stems ?? []);
const pendingStems = computed(() => job.value?.stems.filter(
  (stem) => !outputs.value.some((out) => out.stem === stem && out.path),
) ?? []);
// Una separación recuperada se puede descargar aunque su antigua sesión ya
// no permita consultar el estado: la propia URL de descarga validará el resultado.
const downloadReady = computed(() => status.value?.status === 'done'
  || (job.value?.status === 'restored' && status.value == null));
const publishedCount = presets.reduce((count, item) => count + item.models.filter((m) => !m.is_test).length, 0);
const testCount = presets.reduce((count, item) => count + item.models.filter((m) => m.is_test).length, 0);

watch(group, () => {
  presetId.value = groupedPresets.value[0]?.id ?? '';
});
watch([presetId, showTestModels], () => {
  if (!models.value.some((item) => item.id === modelId.value)) {
    // Elegir el primer modelo publicado y gratuito cuando exista.
    modelId.value = models.value.find((item) => item.access_level === 0)?.id
      ?? models.value[0]?.id ?? '';
  }
});

function isAllowed(item: ModelInfo | null): boolean {
  if (!item) return false;
  const s = session.value;
  if (!s) return true; // Se muestra el catálogo antes de conectar.
  return item.access_level === 0 || item.access_level === 3 || s.is_premium || s.is_tester
    || (item.access_level === 1 && !s.is_sng_country);
}

const accessMessage = computed(() => {
  if (!model.value) return 'Selecciona un modelo de IA.';
  if (!isAllowed(model.value)) return 'Tu cuenta no tiene acceso a este modelo. Selecciona otro o usa una cuenta autorizada.';
  if (model.value.is_test) return 'Modelo interno marcado como prueba en el código de la web; su funcionamiento no está verificado.';
  if (group.value === '6') return 'Separación de seis pistas identificada en el JS; pendiente de una prueba real.';
  if (group.value === '4') return 'Modo Demucs v4 de 4 pistas confirmado con un HAR real; otros modelos requieren pruebas.';
  return 'El servicio determina las pistas según el modelo elegido.';
});

function accessBadge(item: ModelInfo): string {
  const flags = [];
  if (!isAllowed(item)) flags.push('requiere acceso');
  if (item.is_test) flags.push('experimental');
  if (item.heavy) flags.push('pesado');
  return flags.length ? ` · ${flags.join(' · ')}` : '';
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function cancelPolling() {
  generation += 1;
  if (timer) clearTimeout(timer);
  timer = undefined;
  polling.value = false;
}

async function initialize() {
  cancelPolling();
  busy.value = true;
  error.value = '';
  info.value = '';
  session.value = null;
  job.value = null;
  status.value = null;
  outputs.value = [];
  try {
    session.value = await invoke<SessionInfo>('initialize_session', {
      sessionCookie: sessionCookie.value.trim() || null,
    });
    info.value = `Sesión iniciada. Versión del servicio: ${session.value.version}.`;
    sessionCookie.value = '';
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
  }
}

async function selectAudio() {
  error.value = '';
  try {
    const result = await open({
      multiple: false,
      directory: false,
      filters: [{ name: 'Archivos de audio', extensions: ['mp3', 'wav', 'flac', 'm4a', 'ogg'] }],
    });
    if (typeof result === 'string') audioPath.value = result;
  } catch (e) {
    error.value = errorText(e);
  }
}

async function pollStatus(token: number) {
  if (token !== generation) return;
  try {
    const next = await invoke<StatusInfo>('check_status');
    if (token !== generation) return;
    status.value = next;
    if (next.status === 'done') {
      polling.value = false;
      info.value = 'Separación terminada. Selecciona una carpeta para guardar los stems.';
      return;
    }
    if (next.status === 'failed' || next.status === 'error' || next.status === 'rejected') {
      polling.value = false;
      error.value = next.error || 'X-Minus ha rechazado o fallado al procesar el audio.';
      return;
    }
    checks += 1;
    if (checks >= 60) {
      polling.value = false;
      info.value = 'Se ha pausado la consulta automática para respetar al servidor. Puedes reanudarla.';
      return;
    }
    timer = setTimeout(() => { void pollStatus(token); }, 5000);
  } catch (e) {
    if (token === generation) {
      polling.value = false;
      error.value = `No se pudo consultar el estado: ${errorText(e)}`;
    }
  }
}

function resumePolling() {
  if (!job.value) return;
  cancelPolling();
  error.value = '';
  info.value = '';
  status.value = null;
  checks = 0;
  polling.value = true;
  void pollStatus(generation);
}

async function separate() {
  if (!audioPath.value || !session.value || !preset.value || !model.value || !isAllowed(model.value)) return;
  cancelPolling();
  busy.value = true;
  error.value = '';
  info.value = 'Subiendo audio…';
  job.value = null;
  status.value = null;
  outputs.value = [];
  try {
    job.value = await invoke<JobInfo>('start_separation', {
      filePath: audioPath.value,
      presetId: presetId.value,
      modelId: modelId.value,
    });
    info.value = job.value.reused_previous_result
      ? 'X-Minus ha reconocido un trabajo anterior. Comprobando el resultado…'
      : 'Audio recibido. Comprobando el procesamiento cada 5 segundos…';
    resumePolling();
  } catch (e) {
    info.value = '';
    error.value = errorText(e);
  } finally {
    busy.value = false;
  }
}

async function restorePreviousJob() {
  if (!session.value || !model.value || !preset.value || !isAllowed(model.value)) return;
  cancelPolling();
  busy.value = true;
  error.value = '';
  info.value = '';
  outputs.value = [];
  job.value = null;
  status.value = null;
  try {
    job.value = await invoke<JobInfo>('restore_job', {
      jobId: recoveryId.value.trim(),
      sourceFilename: recoveryName.value.trim() || 'audio_recuperado',
      presetId: presetId.value,
      modelId: modelId.value,
    });
    resumePolling();
  } catch (e) {
    error.value = `No se pudo recuperar el trabajo: ${errorText(e)}`;
  } finally {
    busy.value = false;
  }
}

async function download() {
  if (!job.value || !downloadReady.value || pendingStems.value.length === 0) return;
  error.value = '';
  info.value = '';
  try {
    const dir = await open({ directory: true, multiple: false, title: 'Dónde guardar los stems' });
    if (typeof dir !== 'string') return;
    downloading.value = true;
    const fresh = await invoke<StemResult[]>('download_stems', {
      directory: dir,
      requestedStems: pendingStems.value,
    });
    // Conservar los stems descargados anteriormente; solo sustituir los
    // resultados fallidos por los de esta nueva tentativa.
    const previous = outputs.value;
    outputs.value = job.value.stems.map((stem) =>
      fresh.find((item) => item.stem === stem)
      ?? previous.find((item) => item.stem === stem)
      ?? { stem, path: null, error: 'Aún no descargado' },
    );
    const ok = outputs.value.filter((v) => !!v.path).length;
    info.value = `${ok} de ${job.value.stems.length} stems guardados.`;
    if (ok !== job.value.stems.length) {
      error.value = 'Algunos stems no se pudieron descargar. Puedes reintentar únicamente los pendientes.';
    }
  } catch (e) {
    error.value = errorText(e);
  } finally {
    downloading.value = false;
  }
}

onBeforeUnmount(cancelPolling);
</script>

<template>
  <main class="app">
    <header class="header">
      <div class="logo" aria-hidden="true">|||</div>
      <div>
        <h1>X-Minus Stems</h1>
        <p>Wrapper no oficial · Tauri 2 + Vue 3</p>
      </div>
      <span class="pill" :class="{ connected: session }">{{ session ? 'Conectado' : 'Sin conectar' }}</span>
    </header>

    <section class="panel">
      <div class="title-row"><span class="step">01</span><h2>Conectar con X-Minus</h2></div>
      <p class="muted">Se obtiene una clave temporal de <code>x-minus.pro/ai</code>. Las credenciales no quedan guardadas en el proyecto.</p>
      <div class="cookie-row">
        <label class="field-grow">
          <span>Cookie de tu sesión (opcional)</span>
          <input v-model="sessionCookie" type="password" autocomplete="off" placeholder="Probar primero sin cookie" :disabled="locked" />
        </label>
        <button class="button secondary" :disabled="locked" @click="initialize">{{ busy && !session ? 'Conectando…' : 'Inicializar API' }}</button>
      </div>
      <p class="hint">Si tu sesión es necesaria, puedes copiar tu propia cabecera <code>Cookie</code> de una petición al dominio X-Minus. Solo se utiliza en memoria y nunca se envía al CDN.</p>
    </section>

    <section class="panel">
      <div class="title-row"><span class="step">02</span><h2>Audio y separación</h2></div>
      <div class="source-row">
        <button class="button secondary" :disabled="locked" @click="selectAudio">Elegir archivo…</button>
        <span class="path" :title="audioPath">{{ audioPath || 'Ningún archivo seleccionado' }}</span>
      </div>

      <div class="group-header">Número de stems</div>
      <div class="modes">
        <label v-for="item in options" :key="item.id" class="mode" :class="{ selected: group === item.id, disabled: locked }">
          <input v-model="group" type="radio" :value="item.id" :disabled="locked" />
          <span class="mode-title">{{ item.label }}</span>
          <span class="mode-sub">{{ item.subtitle }}</span>
        </label>
      </div>

      <div class="selectors">
        <label class="select-field">
          <span>Tipo de separación</span>
          <select v-model="presetId" :disabled="locked">
            <option v-for="item in groupedPresets" :key="item.id" :value="item.id">{{ item.label }}</option>
          </select>
        </label>
        <label class="select-field">
          <span>Modelo de inteligencia artificial</span>
          <select v-model="modelId" :disabled="locked">
            <option v-for="item in models" :key="item.id" :value="item.id">
              {{ item.name }}{{ accessBadge(item) }}
            </option>
          </select>
        </label>
      </div>

      <p v-if="preset" class="hint">{{ preset.description }}</p>
      <div class="stems-preview">
        <span class="preview-caption">Pistas que descargará este modelo ({{ stems.length }})</span>
        <div class="stem-tags"><span v-for="stem in stems" :key="stem" class="stem-tag">{{ stemLabels[stem] ?? stem }}</span></div>
      </div>
      <p class="model-note" :class="{ 'model-note-warning': model && (!isAllowed(model) || model.is_test) }">{{ accessMessage }}</p>
      <label class="test-toggle">
        <input v-model="showTestModels" type="checkbox" :disabled="locked" />
        Mostrar también los {{ testCount }} modelos de prueba internos (no verificados)
      </label>
      <p class="hint">{{ publishedCount }} entradas publicadas identificadas en el JS {{ catalog.capture_version }}. La disponibilidad depende de tu cuenta y puede cambiar.</p>

      <button class="button primary" :disabled="!session || !audioPath || !model || !isAllowed(model) || locked" @click="separate">
        {{ busy ? 'Subiendo audio…' : `Separar ${stems.length} stems` }}
      </button>
      <details class="recovery">
        <summary>Recuperar un trabajo ya procesado (sin volver a subir audio)</summary>
        <p class="hint">Si ya tienes un ID de trabajo, selecciona su modelo arriba, introduce el ID y consulta su estado. No se vuelve a enviar el MP3.</p>
        <div class="recovery-fields">
          <label class="select-field">
            <span>ID del trabajo anterior</span>
            <input v-model.trim="recoveryId" type="text" placeholder="Pega el job_id" autocomplete="off" spellcheck="false" :disabled="locked" />
          </label>
          <label class="select-field">
            <span>Nombre de la canción (opcional)</span>
            <input v-model="recoveryName" type="text" placeholder="Nombre para los MP3" :disabled="locked" />
          </label>
        </div>
        <button class="button secondary" :disabled="!session || !recoveryId || !model || !isAllowed(model) || locked" @click="restorePreviousJob">
          Recuperar trabajo
        </button>
      </details>
    </section>

    <section v-if="job" class="panel">
      <div class="title-row"><span class="step">03</span><h2>Procesamiento y descargas</h2></div>
      <dl class="details">
        <div><dt>Archivo</dt><dd>{{ job.source_filename }}</dd></div>
        <div><dt>ID del trabajo</dt><dd class="monospace">{{ job.job_id }}</dd></div>
        <div><dt>Estado</dt><dd><strong>{{ status?.status || job.status }}</strong></dd></div>
        <div><dt>Stems solicitados</dt><dd>{{ job.stems.map(stem => stemLabels[stem] ?? stem).join(', ') }}</dd></div>
        <div v-if="status?.remaining_time != null"><dt>Tiempo estimado del servidor</dt><dd>{{ status.remaining_time }} s</dd></div>
        <div v-if="job.requests_left != null"><dt>Solicitudes restantes</dt><dd>{{ job.requests_left }}</dd></div>
      </dl>
      <div class="actions">
        <button v-if="!polling && status?.status !== 'done'" class="button secondary" @click="resumePolling">Volver a consultar</button>
        <button v-if="polling" class="button secondary" @click="cancelPolling">Pausar consultas</button>
        <button class="button primary" :disabled="!downloadReady || downloading || pendingStems.length === 0" @click="download">{{ downloading ? 'Descargando…' : pendingStems.length === 0 ? 'Todos los stems guardados' : outputs.length > 0 ? `Reintentar ${pendingStems.length} stems pendientes` : `Descargar los ${job.stems.length} stems` }}</button>
      </div>
      <p v-if="polling" class="muted">Comprobando cada 5 segundos. El HAR no contiene un porcentaje de procesamiento.</p>
      <p v-if="job.status === 'restored' && !status" class="hint">Trabajo recuperado: si la consulta de estado no responde, puedes intentar descargarlo directamente. El servidor comprobará si el audio está disponible.</p>
      <ul v-if="outputs.length" class="results">
        <li v-for="item in outputs" :key="item.stem">
          <span>{{ stemLabels[item.stem] || item.stem }}</span>
          <span v-if="item.path" class="success-path">{{ item.path }}</span>
          <span v-else class="error-path">{{ item.error }}</span>
        </li>
      </ul>
    </section>

    <p v-if="info" class="notice" role="status">{{ info }}</p>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <footer>API privada susceptible de cambios. El wrapper respeta la autenticación, las cuotas y los permisos del proveedor.</footer>
  </main>
</template>
