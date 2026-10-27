/**
 * Development only (`npm run dev`, then `/?preview=1`): a sample capture so the editor's
 * layout can be checked in a plain browser. Imported dynamically behind
 * `import.meta.env.DEV`, so production builds leave it out.
 */
const WIDTH = 1000;
const HEIGHT = 380;
const SCALE = 2;

export async function loadPreview(show: (image: HTMLImageElement) => void): Promise<void> {
  addTrafficLights();
  const canvas = document.createElement("canvas");
  canvas.width = WIDTH * SCALE;
  canvas.height = HEIGHT * SCALE;
  const ctx = canvas.getContext("2d")!;
  ctx.scale(SCALE, SCALE);
  drawSample(ctx);
  const image = new Image();
  image.src = canvas.toDataURL("image/png");
  await image.decode();
  show(image);
}

/** Stand-ins for the window buttons macOS draws over the title bar. */
function addTrafficLights(): void {
  const lights = document.createElement("div");
  lights.style.cssText = "position:fixed;top:16px;left:14px;display:flex;gap:8px;z-index:100;pointer-events:none";
  for (const color of ["#ff5f57", "#febc2e", "#28c840"]) {
    const dot = document.createElement("span");
    dot.style.cssText = `width:12px;height:12px;border-radius:50%;background:${color};box-shadow:inset 0 0 0 0.5px rgba(0,0,0,.15)`;
    lights.append(dot);
  }
  document.body.append(lights);
}

function drawSample(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = "#f2f2f4";
  ctx.fillRect(0, 0, WIDTH, HEIGHT);
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.roundRect(30, 24, WIDTH - 60, HEIGHT - 24, 12);
  ctx.fill();
  ctx.strokeStyle = "#e3e3e6";
  ctx.stroke();
  const font = "-apple-system, BlinkMacSystemFont, sans-serif";
  ctx.fillStyle = "#1d1d1f";
  ctx.font = `600 22px ${font}`;
  ctx.fillText("Protección", 60, 76);
  ctx.fillStyle = "#6e6e73";
  ctx.font = `16px ${font}`;
  ctx.fillText("Lo que no se ha podido revisar suele necesitar permisos de administrador.", 60, 108);
  const rows = [
    ["fail2ban vigilando SSH", "apt install fail2ban (el jail sshd viene activo en Ubuntu)"],
    ["sudo pide contraseña", "regla NOPASSWD en 90-cloud-init-users (revísala con sudo grep -rn)"],
  ];
  rows.forEach(([title, detail], index) => {
    const top = 150 + index * 105;
    ctx.strokeStyle = "#e5484d";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(70, top - 6, 9, 0, Math.PI * 2);
    ctx.stroke();
    ctx.fillStyle = "#1d1d1f";
    ctx.font = `17px ${font}`;
    ctx.fillText(title, 94, top);
    ctx.fillStyle = "#f2f2f4";
    ctx.beginPath();
    ctx.roundRect(94, top + 14, 600, 40, 6);
    ctx.fill();
    ctx.fillStyle = "#1d1d1f";
    ctx.font = `15px ui-monospace, Menlo, monospace`;
    ctx.fillText(detail, 108, top + 40);
    ctx.fillStyle = "#e5e5ea";
    ctx.fillRect(60, top + 72, WIDTH - 120, 1);
  });
}
