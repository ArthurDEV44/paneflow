"use client";

import { useEffect, useRef } from "react";

const TWEAKS = {
  grainOpacity: 18,
  grainFrequency: 100,
  causticsIntensity: 38,
  contrast: 115,
  animationSpeed: 90,
} as const;

const PALETTE: ReadonlyArray<readonly [number, number, number]> = [
  [18, 14, 56],
  [38, 33, 88],
  [60, 56, 130],
  [92, 96, 180],
  [128, 134, 210],
  [170, 178, 232],
];

const COMPOSITION: ReadonlyArray<ReadonlyArray<number>> = [
  [5, 4, 3, 2],
  [4, 5, 3, 2],
  [3, 4, 4, 2],
  [2, 3, 3, 1],
];

const SUN_HI: readonly [number, number, number] = [255, 238, 190];
const SUN_MID: readonly [number, number, number] = [245, 210, 140];

const CAUSTICS = [
  { x: 18, y: 22, s: 240, blur: 30, c: SUN_HI, o: 0.85, dur: 18 },
  { x: 32, y: 38, s: 160, blur: 50, c: SUN_HI, o: 0.6, dur: 24 },
  { x: 55, y: 55, s: 200, blur: 70, c: SUN_MID, o: 0.45, dur: 22 },
  { x: 78, y: 80, s: 130, blur: 40, c: SUN_MID, o: 0.3, dur: 26 },
] as const;

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const smoothstep = (t: number) => t * t * (3 - 2 * t);

export function MeshHeader() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let phase = 0;
    let raf = 0;
    let mounted = true;
    let renderW = 480;
    let renderH = 107;

    const sizeCanvas = () => {
      const rect = canvas.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      renderW = Math.min(480, Math.max(2, Math.floor(rect.width)));
      renderH = Math.max(2, Math.round(renderW * (rect.height / rect.width)));
      if (canvas.width !== renderW) canvas.width = renderW;
      if (canvas.height !== renderH) canvas.height = renderH;
    };

    const renderMesh = () => {
      const W = renderW;
      const H = renderH;
      const grid = COMPOSITION;
      const palette = PALETTE;
      const rows = grid.length;
      const cols = grid[0].length;
      const t = phase;
      const contrastFactor = TWEAKS.contrast / 100;

      const img = ctx.createImageData(W, H);
      const data = img.data;

      for (let y = 0; y < H; y++) {
        const vy = y / (H - 1);
        for (let x = 0; x < W; x++) {
          const vx = x / (W - 1);

          const wx = vx + Math.sin((vy + t * 0.3) * Math.PI * 1.2) * 0.06;
          const wy = vy + Math.cos((vx + t * 0.2) * Math.PI * 1.3) * 0.05;

          const gx = Math.max(0, Math.min(cols - 1.0001, wx * (cols - 1)));
          const gy = Math.max(0, Math.min(rows - 1.0001, wy * (rows - 1)));
          const x0 = Math.floor(gx);
          const y0 = Math.floor(gy);
          const fx = smoothstep(gx - x0);
          const fy = smoothstep(gy - y0);

          const i00 = grid[y0][x0];
          const i10 = grid[y0][x0 + 1];
          const i01 = grid[y0 + 1][x0];
          const i11 = grid[y0 + 1][x0 + 1];

          const top = lerp(i00, i10, fx);
          const bot = lerp(i01, i11, fx);
          const idx = lerp(top, bot, fy);

          const pi = Math.max(0, Math.min(palette.length - 1.0001, idx));
          const pi0 = Math.floor(pi);
          const pf = pi - pi0;
          const c1 = palette[pi0];
          const c2 = palette[pi0 + 1];

          const o = (y * W + x) * 4;
          for (let k = 0; k < 3; k++) {
            const blended = lerp(c1[k], c2[k], pf);
            const adj = 128 + (blended - 128) * contrastFactor;
            data[o + k] = Math.max(0, Math.min(255, adj));
          }
          data[o + 3] = 255;
        }
      }

      ctx.putImageData(img, 0, 0);
    };

    sizeCanvas();
    renderMesh();

    const animate = () => {
      if (!mounted) return;
      phase += 0.0015 * (TWEAKS.animationSpeed / 100);
      renderMesh();
      raf = requestAnimationFrame(animate);
    };
    raf = requestAnimationFrame(animate);

    const onResize = () => {
      sizeCanvas();
      renderMesh();
    };
    window.addEventListener("resize", onResize);

    return () => {
      mounted = false;
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", onResize);
    };
  }, []);

  const grainURL = (matrix: string, freq: number) => {
    const f = (freq / 100).toFixed(2);
    const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='320' height='320'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='${f}' numOctaves='2' stitchTiles='stitch' seed='7'/><feColorMatrix values='${matrix}'/></filter><rect width='100%' height='100%' filter='url(#n)'/></svg>`;
    return `url("data:image/svg+xml;utf8,${encodeURIComponent(svg)}")`;
  };

  const grainBright = grainURL(
    "0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  0 0 0 1 0",
    TWEAKS.grainFrequency,
  );
  const grainDark = grainURL(
    "0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0",
    TWEAKS.grainFrequency * 1.2,
  );
  const grainOpacity = TWEAKS.grainOpacity / 100;
  const causticsIntensity = TWEAKS.causticsIntensity / 100;
  const speedFactor =
    TWEAKS.animationSpeed > 0 ? 100 / TWEAKS.animationSpeed : 1;

  return (
    <section
      className="relative w-full overflow-hidden rounded-2xl border border-surface-border isolate"
      style={{ aspectRatio: "1440 / 320", background: "#0f0b2e" }}
    >
      <style>{`
        @keyframes meshChromaShift {
          0%, 100% { transform: translate(0, 0); }
          50% { transform: translate(-2%, 3%); }
        }
        @keyframes meshGrainShift {
          0%   { transform: translate(0, 0); }
          12%  { transform: translate(-4%, -2%); }
          25%  { transform: translate(-6%, 3%); }
          37%  { transform: translate(2%, -5%); }
          50%  { transform: translate(-3%, 4%); }
          62%  { transform: translate(5%, 2%); }
          75%  { transform: translate(-5%, -3%); }
          87%  { transform: translate(3%, -6%); }
          100% { transform: translate(0, 0); }
        }
        @keyframes meshCaustic1 {
          0%   { transform: translate(-50%, -50%) scale(1); opacity: 1; }
          100% { transform: translate(calc(-50% + 28px), calc(-50% + 14px)) scale(1.2); opacity: 0.85; }
        }
        @keyframes meshCaustic2 {
          0%   { transform: translate(-50%, -50%) scale(1); opacity: 0.9; }
          100% { transform: translate(calc(-50% - 22px), calc(-50% + 18px)) scale(1.15); opacity: 1; }
        }
        @keyframes meshCaustic3 {
          0%   { transform: translate(-50%, -50%) scale(1); opacity: 0.7; }
          100% { transform: translate(calc(-50% + 24px), calc(-50% - 16px)) scale(1.25); opacity: 1; }
        }
        @keyframes meshCaustic4 {
          0%   { transform: translate(-50%, -50%) scale(1); opacity: 0.5; }
          100% { transform: translate(calc(-50% - 18px), calc(-50% - 20px)) scale(1.3); opacity: 0.8; }
        }
      `}</style>

      <canvas ref={canvasRef} className="absolute inset-0 block w-full h-full" />

      <div
        className="pointer-events-none absolute inset-0 z-[2]"
        style={{ mixBlendMode: "screen" }}
      >
        {CAUSTICS.map((p, i) => {
          const [r, g, b] = p.c;
          return (
            <div
              key={i}
              className="absolute rounded-full"
              style={{
                left: `${p.x}%`,
                top: `${p.y}%`,
                width: `${p.s}px`,
                height: `${p.s}px`,
                transform: "translate(-50%, -50%)",
                filter: `blur(${p.blur}px)`,
                background: `radial-gradient(circle at 50% 50%, rgba(${r},${g},${b},${(p.o * causticsIntensity).toFixed(2)}) 0%, rgba(${r},${g},${b},${(p.o * causticsIntensity * 0.4).toFixed(2)}) 30%, transparent 70%)`,
                animation: `meshCaustic${i + 1} ${(p.dur * speedFactor).toFixed(1)}s ease-in-out infinite alternate`,
                willChange: "transform, opacity",
              }}
            />
          );
        })}
      </div>

      <div
        className="pointer-events-none absolute inset-0 z-[3]"
        style={{
          mixBlendMode: "screen",
          background:
            "radial-gradient(42% 52% at 28% 22%, rgba(255, 228, 165, 0.18) 0%, rgba(255, 210, 140, 0.08) 40%, transparent 70%), radial-gradient(30% 40% at 70% 80%, rgba(150, 140, 220, 0.06) 0%, transparent 70%)",
          animation: "meshChromaShift 18s ease-in-out infinite alternate",
        }}
      />

      <div
        className="pointer-events-none absolute inset-0 z-[4]"
        style={{
          background:
            "radial-gradient(ellipse 90% 70% at 50% 50%, transparent 35%, rgba(15, 11, 46, 0.15) 65%, rgba(15, 11, 46, 0.55) 90%, rgba(15, 11, 46, 0.8) 100%)",
        }}
      />

      <div
        className="pointer-events-none absolute z-10"
        style={{
          inset: "-25%",
          width: "150%",
          height: "150%",
          opacity: grainOpacity,
          mixBlendMode: "overlay",
          backgroundImage: grainBright,
          backgroundSize: "320px 320px",
          animation: "meshGrainShift 0.8s steps(8) infinite",
        }}
      />

      <div
        className="pointer-events-none absolute z-[11]"
        style={{
          inset: "-25%",
          width: "150%",
          height: "150%",
          opacity: grainOpacity * 0.6,
          mixBlendMode: "multiply",
          backgroundImage: grainDark,
          backgroundSize: "320px 320px",
          animation: "meshGrainShift 1s steps(8) infinite reverse",
        }}
      />

      <div className="relative z-20 flex h-full items-center px-6 sm:px-10">
        <h2 className="m-0 font-semibold leading-[0.98] tracking-tight text-[clamp(28px,5.2vw,64px)] text-white">
          Téléchargements ouverts.{" "}
          <em className="not-italic font-normal opacity-65">Linux disponible.</em>
        </h2>
      </div>
    </section>
  );
}
