import { extension } from "./format";

// The drag plugin wants a PNG data URL for the image under the cursor.
export function dragIcon(name: string): string {
  const size = 64;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d")!;
  const gradient = ctx.createLinearGradient(0, 0, size, size);
  gradient.addColorStop(0, "#6d5cff");
  gradient.addColorStop(1, "#2db8f0");
  ctx.fillStyle = gradient;
  ctx.beginPath();
  ctx.roundRect(8, 4, 48, 56, 8);
  ctx.fill();
  ctx.fillStyle = "#fff";
  ctx.font = "600 13px -apple-system, 'Segoe UI', sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(extension(name), size / 2, size / 2 + 2);
  return canvas.toDataURL("image/png");
}
