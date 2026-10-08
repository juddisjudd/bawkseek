// bun scripts/themes.ts <bawkterm checkout> > assets/themes/presets.json
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const root = process.argv[2]
if (!root) throw new Error('usage: bun scripts/themes.ts <bawkterm checkout>')
const lib = join(root, 'src/renderer/src/lib/theme.ts')
const { TERMINAL_THEMES, appColors } = await import(lib)

const KEYS = [
  'bg', 'bg-weak', 'bg-weak-hover', 'bg-strong', 'bg-strong-hover', 'bg-interactive', 'bg-selected',
  'text', 'text-weak', 'text-weaker', 'text-strong', 'text-on-interactive', 'border', 'border-weak',
  'icon', 'focus', 'danger', 'success', 'warning', 'accent',
  'folder-red', 'folder-orange', 'folder-yellow', 'folder-green', 'folder-blue', 'folder-purple'
]

const presets = TERMINAL_THEMES.filter((t: any) => !t.builtin).map((t: any) => {
  const app = appColors(t.id)!
  const c = t.colors
  return {
    id: t.id,
    label: t.label,
    dark: t.dark,
    card: {
      bg: c.background,
      fg: c.foreground,
      cursor: c.cursor ?? c.foreground,
      swatches: [c.red, c.green, c.yellow, c.blue, c.magenta, c.cyan]
    },
    colors: Object.fromEntries(KEYS.map((k) => [k, app[`--${k}`]]))
  }
})

// the app's own palette is written in OKLCH in app.css
function oklch(l: number, c: number, h: number): string {
  const a = c * Math.cos((h * Math.PI) / 180)
  const b = c * Math.sin((h * Math.PI) / 180)
  const lc = (l + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const mc = (l - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const sc = (l - 0.0894841775 * a - 1.291485548 * b) ** 3
  const linear = [
    4.0767416621 * lc - 3.3077115913 * mc + 0.2309699292 * sc,
    -1.2684380046 * lc + 2.6097574011 * mc - 0.3413193965 * sc,
    -0.0041960863 * lc - 0.7034186147 * mc + 1.707614701 * sc
  ]
  const channel = (v: number): string => {
    const x = Math.min(1, Math.max(0, v))
    const srgb = x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055
    return Math.round(srgb * 255).toString(16).padStart(2, '0')
  }
  return '#' + linear.map(channel).join('')
}

if (process.argv[3] === '--builtin') {
  const css = readFileSync(join(root, 'src/renderer/src/app.css'), 'utf8')
  for (const block of css.split(/\r?\n}\r?\n/).slice(0, 2)) {
    const out: Record<string, string> = {}
    for (const m of block.matchAll(/--([\w-]+): oklch\(([\d.]+) ([\d.]+) ([\d.]+)\)/g)) {
      out[m[1]] = oklch(+m[2], +m[3], +m[4])
    }
    console.log(JSON.stringify(out, null, 2))
  }
} else {
  console.log(JSON.stringify(presets, null, 2))
}
