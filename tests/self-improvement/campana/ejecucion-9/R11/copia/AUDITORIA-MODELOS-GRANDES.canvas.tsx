//! Copia de archivo de la auditoría de modelos grandes (actualizada 2026-07-21).
//! El Canvas vivo de Cursor está en:
//!   ~/.cursor/projects/home-jmdiez-Trabajo-demonodojo-soso/canvases/soso-modelos-grandes.canvas.tsx
//! Este fichero es para versionar el análisis en el repo; no se abre como Canvas
//! desde aquí (Cursor solo carga `.canvas.tsx` de la carpeta de canvases del IDE).

import {
  Callout,
  Card,
  CardBody,
  CardHeader,
  Code,
  Divider,
  Grid,
  H1,
  H2,
  H3,
  Pill,
  Row,
  Stack,
  Stat,
  Table,
  Text,
  useHostTheme,
} from "cursor/canvas";

type CapStatus = "listo" | "parcial" | "bloqueado" | "stub";

const CAPACIDADES: Array<{
  area: string;
  status: CapStatus;
  detalle: string;
  evidencia: string;
}> = [
  {
    area: "Memoria / mmap",
    status: "listo",
    detalle: "Ventana VA ~416 GiB (64–480 GiB); USER_MAX 512 GiB",
    evidencia: "soso-abi MMAP_BASE/LIMIT; addrspace.rs USER_MAX",
  },
  {
    area: "Zero-copy pesos",
    status: "listo",
    detalle: "tensor_view + matvec fusionado F32/Q8_0/Q4_K; sin copia por token",
    evidencia: "soso-llm-core layer.rs / source.rs",
  },
  {
    area: "Huge pages 2 MiB",
    status: "listo",
    detalle: "Fault alineado → allocate_2m + read_range_direct",
    evidencia: "task/mod.rs handle_mmap_fault",
  },
  {
    area: "Frame allocator",
    status: "listo",
    detalle: "Cursor O(1) + free-list + bloques 2 MiB",
    evidencia: "mm/frame.rs",
  },
  {
    area: "Q4_K + sampling + streaming",
    status: "listo",
    detalle: "Runtime llama completo; TinyLlama verificado end-to-end",
    evidencia: "PLAN L2; soso-llm run",
  },
  {
    area: "SIMD AVX2+FMA",
    status: "listo",
    detalle: "Target userspace propio; xsave64 en kernel",
    evidencia: "x86_64-soso-user.json; gemm.rs; arch/fpu.rs",
  },
  {
    area: "Arranque SMP (L3a)",
    status: "listo",
    detalle: "ACPI MADT + LAPIC + APs en modo largo (SMP=4/8 verde)",
    evidencia: "arch/smp.rs; arch/apic.rs",
  },
  {
    area: "Scheduler multicore (L3b)",
    status: "listo",
    detalle: "APs ejecutan procesos reales; schedule() despacha por cpu_index",
    evidencia: "701c170; ap_enter_scheduler desde ap_entry; ~20× test limpio",
  },
  {
    area: "Threads / futex / GEMM paralelo",
    status: "bloqueado",
    detalle: "Procesos migran entre cores, pero decode = 1 hilo lógico",
    evidencia: "soso-abi sin thread_spawn; gemm secuencial",
  },
  {
    area: "GPU real",
    status: "stub",
    detalle: "Stub Intel: VRAM = Vec en RAM; submit no-op; sin NVIDIA",
    evidencia: "drivers/gpu.rs; crates/soso-gpu",
  },
  {
    area: "Hardware físico",
    status: "bloqueado",
    detalle: "Solo BIOS + virtio en QEMU; sin UEFI/NVMe/NIC real en xtask",
    evidencia: "xtask create_bios_image",
  },
];

const CONCURRENCIA: Array<{
  id: string;
  titulo: string;
  clase: "confirmado" | "hipotesis" | "descartado" | "futuro";
  prioridad: string;
  donde: string;
  nota: string;
}> = [
  {
    id: "P0-stack",
    titulo: "schedule() siempre usaba pila BSP",
    clase: "descartado",
    prioridad: "era P0",
    donde: "task/mod.rs::schedule → schedule_landing",
    nota: "Causa raíz de las corrupciones intermitentes. Un AP en block/exit saltaba a KSTACK de la BSP. Fix: cpu_index()==0 → schedule_landing, else ap_schedule_landing. Verificado ~20× SMP=1/4/8.",
  },
  {
    id: "P0-kill",
    titulo: "kill_pid + cleanup UAF de AddrSpace",
    clase: "confirmado",
    prioridad: "P0",
    donde: "task/mod.rs kill_pid, schedule_inner, timer_tick",
    nota: "Sigue abierto: marca Zombie sin parar el core dueño; cleanup puede free() mientras ring 3 sigue; timer puede Zombie→Runnable. Más peligroso ahora con SMP real.",
  },
  {
    id: "P1-virtio",
    titulo: "Virtio sin lock global entre blk0/blk1/net",
    clase: "hipotesis",
    prioridad: "P2",
    donde: "virtio_hal + BLK0/BLK1/NET Mutex independientes",
    nota: "DMA_FREE TOCTOU ya corregido. Tras el fix de schedule no hay evidencia activa en ~20 runs; auditoría preventiva recomendada, no bloqueante.",
  },
  {
    id: "P1-tlb",
    titulo: "Sin TLB shootdown cross-CPU",
    clase: "futuro",
    prioridad: "P1",
    donde: "addrspace.rs map_page .ignore()",
    nota: "Hoy un proceso = un PML4 y CR3 se recarga al migrar. Crítico cuando haya threads que compartan AddrSpace.",
  },
  {
    id: "P2-sbrk",
    titulo: "SbrkAllocator: 1 syscall por alloc pequeña",
    clase: "confirmado",
    prioridad: "P2",
    donde: "user/libsoso SbrkAllocator; schedule_inner sondeo AP",
    nota: "~15000 sbrk en soso-llm tiny → contención PROCS con APs ociosos. No es cuelgue; test ssh_llm subido a 100s. Mejora: arena local o wake por IPI.",
  },
  {
    id: "deadlock-sti",
    titulo: "Deadlock sti + PROCS en timer",
    clase: "descartado",
    prioridad: "—",
    donde: "syscall_entry sti; timer_tick",
    nota: "Si el timer interrumpe ring 0, timer_tick retorna en cs&3!=3 antes de net::poll o PROCS.lock().",
  },
];

const PLAN_STALE: Array<{ afirmacion: string; realidad: string }> = [
  {
    afirmacion: "Ventana mmap ~448 GiB / diagnóstico 1 GiB",
    realidad: "Código: 416 GiB (0x10_0000_0000 … 0x78_0000_0000). Límite 1 GiB obsoleto.",
  },
  {
    afirmacion: "Subir BRK_MAX",
    realidad: "BRK_MAX sigue en 0x6000_0000; escaló USER_MAX + mmap, no el heap clásico.",
  },
  {
    afirmacion: "DMA virtio directo al frame",
    realidad: "Hay lectura directa al buffer del frame, pero sigue siendo copia driver→RAM, no DMA hardware.",
  },
  {
    afirmacion: "xtask produce BIOS + UEFI",
    realidad: "Solo create_bios_image en xtask hoy.",
  },
  {
    afirmacion: "Diagnóstico: sin APIC / sin APs / L3b aparcado",
    realidad: "L3a+L3b scheduler operativo (701c170). Pendiente: threads/GEMM, kill_pid, L5/L6.",
  },
  {
    afirmacion: "Verificación 30–60 GB en QEMU -m 96G",
    realidad: "Documentado el caso 1.34 GiB; no hay evidencia en repo de la prueba 30–60 GB.",
  },
];

type RowTone = "success" | "warning" | "danger" | "neutral" | "info";
type PillTone = "success" | "warning" | "info" | "neutral" | "deleted";

function statusRowTone(s: CapStatus): RowTone {
  switch (s) {
    case "listo":
      return "success";
    case "parcial":
      return "warning";
    case "bloqueado":
      return "danger";
    case "stub":
      return "info";
  }
}

function statusPillTone(s: CapStatus): PillTone {
  switch (s) {
    case "listo":
      return "success";
    case "parcial":
      return "warning";
    case "bloqueado":
      return "deleted";
    case "stub":
      return "info";
  }
}

function statusLabel(s: CapStatus): string {
  switch (s) {
    case "listo":
      return "Listo";
    case "parcial":
      return "Parcial";
    case "bloqueado":
      return "Bloqueado";
    case "stub":
      return "Stub";
  }
}

function claseRowTone(c: (typeof CONCURRENCIA)[0]["clase"]): RowTone {
  switch (c) {
    case "confirmado":
      return "danger";
    case "hipotesis":
      return "warning";
    case "descartado":
      return "success";
    case "futuro":
      return "info";
  }
}

function clasePillTone(c: (typeof CONCURRENCIA)[0]["clase"]): PillTone {
  switch (c) {
    case "confirmado":
      return "deleted";
    case "hipotesis":
      return "warning";
    case "descartado":
      return "success";
    case "futuro":
      return "info";
  }
}

function claseLabel(c: (typeof CONCURRENCIA)[0]["clase"]): string {
  switch (c) {
    case "confirmado":
      return "Confirmado";
    case "hipotesis":
      return "Hipótesis";
    case "descartado":
      return "Resuelto";
    case "futuro":
      return "Futuro";
  }
}

export default function SosoModelosGrandes() {
  const theme = useHostTheme();

  return (
    <Stack gap={28} style={{ padding: 24, maxWidth: 1100 }}>
      <Stack gap={8}>
        <H1>soso — capacidad para modelos grandes</H1>
        <Text tone="secondary">
          Auditoría actualizada tras 701c170 (fixed multicore errors).
          Objetivo: 70B+ Q4_K con todos los cores y GPUs. Fuente: HEAD +
          PLAN-MODELOS-GRANDES.md.
        </Text>
        <Row gap={8} wrap>
          <Pill tone="success" active>
            L1 memoria
          </Pill>
          <Pill tone="success" active>
            L2 Q4_K
          </Pill>
          <Pill tone="success" active>
            L4 SIMD
          </Pill>
          <Pill tone="success" active>
            L3a APs online
          </Pill>
          <Pill tone="success" active>
            L3b scheduler
          </Pill>
          <Pill tone="warning" active>
            sin threads LLM
          </Pill>
          <Pill tone="neutral" active>
            L5/L6 pendientes
          </Pill>
        </Row>
      </Stack>

      <Callout tone="success" title="SMP usable: APs ejecutan procesos">
        Causa raíz de las corrupciones: <Code>schedule()</Code> iba siempre a{" "}
        <Code>schedule_landing()</Code> (pila BSP). Un proceso que se bloqueaba
        o salía en un AP pisaba la pila de la BSP. Fix: despacho por{" "}
        <Code>percpu::cpu_index()</Code>. Con eso,{" "}
        <Code>ap_enter_scheduler()</Code> está activo; procesos migran entre
        cores (p. ej. cpu=3 → 0 → 2). ~20× <Code>cargo xtask test</Code> en
        SMP=1/4/8 sin panics.
      </Callout>

      <Callout tone="warning" title="Techo de rendimiento LLM hoy">
        El scheduler multicore reparte procesos, pero soso-llm sigue siendo
        monohilo: un solo matvec AVX2 por token. Para saturar N cores hace
        falta thread_spawn + GEMM paralelo. GPU = stub. 70B sigue viable en
        memoria; el multiplicador de tok/s aún no está explotado.
      </Callout>

      <Grid columns={4} gap={12}>
        <Stat value="N" label="Cores con procesos" tone="success" />
        <Stat value="1" label="Hilos por inferencia" tone="warning" />
        <Stat value="416 GiB" label="Ventana mmap VA" tone="success" />
        <Stat value="0" label="Kernels GPU reales" tone="danger" />
      </Grid>

      <Divider />

      <Stack gap={12}>
        <H2>1. Estado de capacidades</H2>
        <Text tone="secondary">
          Qué está demostrado en código frente a lo pendiente para 70B usable.
        </Text>
        <Table
          headers={["Área", "Estado", "Detalle", "Evidencia"]}
          columnAlign={["left", "left", "left", "left"]}
          rowTone={CAPACIDADES.map((c) => statusRowTone(c.status))}
          rows={CAPACIDADES.map((c) => [
            c.area,
            <Pill tone={statusPillTone(c.status)} size="sm">
              {statusLabel(c.status)}
            </Pill>,
            c.detalle,
            <Text size="small" tone="tertiary">
              {c.evidencia}
            </Text>,
          ])}
          striped
          stickyHeader
        />
      </Stack>

      <Grid columns={2} gap={16}>
        <Card>
          <CardHeader>SMP efectivo (actualizado)</CardHeader>
          <CardBody>
            <Stack gap={10}>
              <Text>
                <Code>ap_entry</Code> llama a{" "}
                <Code>task::ap_enter_scheduler()</Code> — ya no hay loop{" "}
                <Code>hlt</Code>.
              </Text>
              <Text weight="semibold">
                schedule() → landing del core actual (BSP o AP vía GS)
              </Text>
              <Text tone="secondary" size="small">
                Fixes previos en el camino: DMA_FREE TOCTOU (un solo lock);
                pilas AP 64 KiB. Red/PIT siguen en BSP; APs hacen cómputo y
                syscalls.
              </Text>
              <Text tone="secondary" size="small">
                Restante L3b: wake por IPI (menos sondeo), auditoría
                preventiva net/fs/drivers, threads + GEMM.
              </Text>
            </Stack>
          </CardBody>
        </Card>
        <Card>
          <CardHeader>GPU y multi-GPU</CardHeader>
          <CardBody>
            <Stack gap={10}>
              <Text>
                Driver Intel detecta PCI class 0x03 y estima VRAM;{" "}
                <Code>alloc</Code> reserva <Code>Vec&lt;u8&gt;</Code> en heap
                del kernel; <Code>submit</Code> retorna OK sin computar.
              </Text>
              <Text>
                <Code>soso-gpu::gemm_f32</Code> siempre false.{" "}
                <Code>Runtime.backend</Code> / TierManager no se usan en el
                forward de soso-llm.
              </Text>
              <Text tone="secondary" size="small">
                Sin NVIDIA, sin GSP/OpenRM, sin multi-GPU. El stub no es un
                camino incremental hacia cómputo real.
              </Text>
            </Stack>
          </CardBody>
        </Card>
      </Grid>

      <Card>
        <CardHeader>Runtime LLM (CPU)</CardHeader>
        <CardBody>
          <Stack gap={8}>
            <Grid columns={3} gap={12}>
              <Stack gap={4}>
                <Text weight="semibold">Listo</Text>
                <Text size="small" tone="secondary">
                  RoPE, GQA, Wo, SwiGLU, Q4_K/Q8_0, KV f16, sampling temp/top-p,
                  streaming SSH, validate_shapes
                </Text>
              </Stack>
              <Stack gap={4}>
                <Text weight="semibold">No usado</Text>
                <Text size="small" tone="secondary">
                  Backend::Gpu, TierManager.prefetch (cola sin I/O),
                  schedule_prefetch en forward
                </Text>
              </Stack>
              <Stack gap={4}>
                <Text weight="semibold">Monohilo</Text>
                <Text size="small" tone="secondary">
                  matvec por filas en un solo hilo; el proceso puede migrar de
                  core, pero no reparte filas
                </Text>
              </Stack>
            </Grid>
            <Text size="small" tone="tertiary">
              Decode 70B Q4 (~40 GB) es memory-bound: techo teórico ~1.5–10
              tok/s con N cores + DRAM ancha. Hoy: 1× AVX2 aunque el proceso
              corra en un AP.
            </Text>
          </Stack>
        </CardBody>
      </Card>

      <Divider />

      <Stack gap={12}>
        <H2>2. Concurrencia: evidencia vs hipótesis</H2>
        <Text tone="secondary">
          Actualizado tras encontrar la causa raíz de la carrera residual.
        </Text>
        <Table
          headers={["ID", "Hallazgo", "Clase", "Pri", "Dónde", "Nota"]}
          rowTone={CONCURRENCIA.map((c) => claseRowTone(c.clase))}
          rows={CONCURRENCIA.map((c) => [
            c.id,
            c.titulo,
            <Pill tone={clasePillTone(c.clase)} size="sm">
              {claseLabel(c.clase)}
            </Pill>,
            c.prioridad,
            <Text size="small">{c.donde}</Text>,
            <Text size="small" tone="secondary">
              {c.nota}
            </Text>,
          ])}
          striped
          stickyHeader
        />
      </Stack>

      <Grid columns={2} gap={16}>
        <Callout tone="success" title="Resuelto: corrupción de pila SMP">
          <Stack gap={6}>
            <Text size="small">
              Instrumentación mostró sosh (pid 2) en cpu=2 hasta bloquearse
              en stdin — entonces <Code>block_current → schedule()</Code>{" "}
              saltaba a la pila BSP. Síntomas aleatorios (virtio, smoltcp,
              curve25519) eran corrupción secundaria de esa pila compartida.
            </Text>
            <Text size="small">
              Commit <Code>701c170</Code>. Ya no es hipótesis de virtio/crypto.
            </Text>
          </Stack>
        </Callout>
        <Callout tone="danger" title="Abierto: ciclo de vida de procesos">
          <Stack gap={6}>
            <Text size="small">
              <Code>kill_pid</Code> pone Zombie + parent=0 sin IPI ni parada
              del owner_cpu. El cleanup de zombis huérfanos puede liberar
              AddrSpace mientras el proceso sigue en ring 3. timer_tick puede
              forzar Runnable sin mirar Zombie.
            </Text>
            <Text size="small">
              Con SMP real es más urgente: shell SSH en AP + teardown en BSP.
            </Text>
          </Stack>
        </Callout>
      </Grid>

      <Card>
        <CardHeader trailing={<Pill tone="info">Deuda</Pill>}>
          Contención benigna (no corrupción)
        </CardHeader>
        <CardBody>
          <Stack gap={8}>
            <Text size="small">
              APs ociosos despiertan cada ~10 ms (timer LAPIC) a mirar PROCS.
              Combinado con ~15000 sbrk del allocator userspace, el test
              ssh_llm a veces necesita &gt;45 s bajo TCG (margen subido a
              100 s). El sistema termina bien; no es deadlock.
            </Text>
            <Text size="small">
              Mejoras futuras: arena en SbrkAllocator; wake de APs por IPI
              cuando hay trabajo nuevo. Reconexión SSH rápida del arnés:
              preexistente, no relacionada con SMP.
            </Text>
          </Stack>
        </CardBody>
      </Card>

      <Divider />

      <Stack gap={12}>
        <H2>3. Arquitectura objetivo</H2>
        <Text tone="secondary">
          Fase B (scheduler AP) hecha. Prioridad: invariantes de kill, luego
          threads para saturar cores en el matvec.
        </Text>
      </Stack>

      <Grid columns={2} gap={14}>
        <Card>
          <CardHeader trailing={<Pill tone="deleted">Urgente</Pill>}>
            Fase A — propiedad de procesos
          </CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text size="small">
                owner_cpu / KillPending; IPI de parada remota; no liberar
                AddrSpace mientras algún current_pid lo referencia.
              </Text>
              <Text size="small">
                timer_tick: solo Running→Runnable; nunca tocar Zombie /
                Waiting*.
              </Text>
              <Text size="small">
                Unificar entradas BSP/AP sobre GS (USER_RSP_SCRATCH /
                TIMER_FPU / KSTACK fijos) — deuda de simetría, no bloqueante.
              </Text>
            </Stack>
          </CardBody>
        </Card>
        <Card>
          <CardHeader trailing={<Pill tone="success">Hecho</Pill>}>
            Fase B — scheduler AP
          </CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text size="small">
                ap_enter_scheduler activo; schedule() por cpu_index; estrés
                SMP=4/8 estable.
              </Text>
              <Text size="small">
                Restante opcional: wake por IPI, auditoría preventiva
                virtio/net/fs, instrumentación cpu/pid en panic.
              </Text>
              <Text size="small">
                Lock VIRTIO global ya no es prerequisito demostrado; solo
                herramienta si reaparecen síntomas de E/S.
              </Text>
            </Stack>
          </CardBody>
        </Card>
        <Card>
          <CardHeader trailing={<Pill tone="info">Throughput</Pill>}>
            Fase C — threads + GEMM paralelo
          </CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text size="small">
                thread_spawn (mismo PML4, pila nueva) + futex wait/wake;
                workers persistentes por token.
              </Text>
              <Text size="small">
                TLB shootdown (IPI invlpg) al mutar page tables compartidas.
              </Text>
              <Text size="small">
                Repartir filas de matvec en gemm.rs; barrera por token.
                Escalar tok/s hasta saturar canales DRAM.
              </Text>
            </Stack>
          </CardBody>
        </Card>
        <Card>
          <CardHeader trailing={<Pill tone="neutral">Fuera de QEMU</Pill>}>
            Fase D — hw real + spike GPU
          </CardHeader>
          <CardBody>
            <Stack gap={6}>
              <Text size="small">
                L5: UEFI, NVMe, NIC física, consola serie/GOP — independiente
                del cómputo.
              </Text>
              <Text size="small">
                L6: spike NVIDIA go/no-go (GSP, nouveau/NVK, SASS). No
                extender el stub Intel; es otro proyecto (meses).
              </Text>
              <Text size="small">
                70B usable en servidor: depende de Fase C (threads), no de
                GPU.
              </Text>
            </Stack>
          </CardBody>
        </Card>
      </Grid>

      <Card>
        <CardHeader>Flujo de dependencias</CardHeader>
        <CardBody>
          <Stack gap={6}>
            <Text size="small" weight="semibold">
              Kill/UAF fix → threads/futex → GEMM paralelo → (paralelo) L5 hw /
              L6 GPU spike
            </Text>
            <Text size="small" tone="secondary">
              L1/L2/L4/L3b scheduler ya están detrás. El siguiente
              multiplicador de tok/s es C, no más drivers virtio.
            </Text>
            <div
              style={{
                marginTop: 8,
                padding: 12,
                background: theme.fill.tertiary,
                borderRadius: 6,
                fontFamily: "ui-monospace, monospace",
                fontSize: 12,
                color: theme.text.secondary,
                lineHeight: 1.6,
                whiteSpace: "pre-wrap",
              }}
            >
              {`[L1]──[L2]──[L4]──[L3b scheduler ✓]──► (hoy)
                              │
                    [A kill/UAF]──[C threads+GEMM]
                              │
                    [D hw]    [D GPU spike]`}
            </div>
          </Stack>
        </CardBody>
      </Card>

      <Divider />

      <Stack gap={12}>
        <H2>4. Puertas de validación</H2>
        <Table
          headers={["Fase", "Prueba", "Criterio de avance"]}
          rows={[
            [
              "B ✓",
              "cargo xtask test SOSO_QEMU_SMP=1/4/8 × ~20",
              "Cumplido: cero panics/corrupciones tras fix schedule()",
            ],
            [
              "A",
              "SSH disconnect durante proceso Running (SMP=4)",
              "Sin UAF; proceso muere en el core dueño; AddrSpace liberado solo tras parada",
            ],
            [
              "B+",
              "Wake por IPI / arena sbrk",
              "ssh_llm estable en 45s con SMP=8 bajo TCG",
            ],
            [
              "C",
              "init test threads + estrés FPU multicore",
              "YMM íntegros; barreras futex correctas",
            ],
            [
              "C",
              "Modelo sintético grande: tok/s vs N workers",
              "Escalado ~lineal hasta saturar RAM bandwidth",
            ],
            [
              "D",
              "Spike NVIDIA 2 semanas",
              "Go/no-go documentado; no bloquear path CPU",
            ],
          ]}
          striped
        />
      </Stack>

      <Stack gap={12}>
        <H2>5. Afirmaciones del plan desactualizadas</H2>
        <Text tone="secondary" size="small">
          Contrastar PLAN-MODELOS-GRANDES.md (tabla diagnóstico + L5 UEFI)
          con el código. La sección L3b del plan ya documenta el fix.
        </Text>
        <Table
          headers={["Afirmación del plan", "Realidad en código"]}
          rows={PLAN_STALE.map((p) => [p.afirmacion, p.realidad])}
          striped
        />
      </Stack>

      <Divider />

      <Stack gap={10}>
        <H3>Conclusión</H3>
        <Text>
          soso ya tiene memoria grande, SIMD y scheduler SMP real: los APs
          ejecutan procesos y migran trabajo entre cores. El multiplicador
          que falta para 70B usable es threads + GEMM paralelo (un proceso
          AVX2 no satura N canales de DRAM). El riesgo abierto más serio es
          kill_pid/UAF bajo SMP; la carrera de corrupción de pila quedó
          cerrada. GPU NVIDIA sigue siendo un spike aparte, no una extensión
          del stub Intel.
        </Text>
        <Text tone="tertiary" size="small">
          Canvas de auditoría · actualizado 2026-07-21 tras 701c170 · copia en
          AUDITORIA-MODELOS-GRANDES.canvas.tsx
        </Text>
      </Stack>
    </Stack>
  );
}
