import { FacetryError } from "./errors.js";
import type { IconSource } from "./icons.js";

/** One icon of an Iconify JSON set: the SVG markup inside its `<svg>`, and its box. */
export interface IconifyIcon {
  body: string;
  left?: number;
  top?: number;
  width?: number;
  height?: number;
  rotate?: number;
  hFlip?: boolean;
  vFlip?: boolean;
}

/**
 * An icon set in Iconify's JSON format (`@iconify-json/*` packages, `icons.json`), as Mermaid's
 * `registerIconPacks` takes it: icons by name, aliases of them, and the default box.
 */
export interface IconifyJSON {
  prefix?: string;
  icons: Record<string, IconifyIcon>;
  aliases?: Record<string, { parent: string; rotate?: number; hFlip?: boolean; vFlip?: boolean }>;
  left?: number;
  top?: number;
  width?: number;
  height?: number;
}

/** What a pack holds: an Iconify JSON set, or icons by name as `DiagramOptions.icons` takes them. */
export type IconPackIcons = IconifyJSON | Record<string, IconSource>;

/**
 * A named set of icons for {@link diagram.registerIconPacks}: `@{ icon: "<name>:<icon>" }` in
 * Mermaid text draws its icons. Give the icons, or a `loader` that fetches them the first time a
 * diagram names one.
 */
export type IconPack =
  | { name: string; icons: IconPackIcons }
  | { name: string; loader: () => Promise<IconPackIcons> };

interface Registered {
  pack: IconPack;
  /** The loaded icons, once a loader has run (or at once for given icons). */
  loaded?: Promise<IconPackIcons>;
}

const packs = new Map<string, Registered>();

/** Adds icon packs, or replaces packs of the same names. */
export function registerIconPacks(list: IconPack[]): void {
  for (const pack of list) {
    if (!/^[A-Za-z0-9_-]+$/.test(pack.name)) {
      throw new FacetryError(
        "invalid-options",
        `icon pack name ${JSON.stringify(pack.name)} must be letters, digits, \`-\` or \`_\``,
      );
    }
    packs.set(pack.name, { pack });
  }
}

/** The names of the registered icon packs. */
export function iconPackNames(): string[] {
  return [...packs.keys()];
}

function isIconify(icons: IconPackIcons): icons is IconifyJSON {
  const inner = (icons as IconifyJSON).icons;
  if (typeof inner !== "object" || inner === null || Array.isArray(inner)) return false;
  const first = Object.values(inner)[0];
  return first === undefined || typeof (first as IconifyIcon).body === "string";
}

function load(entry: Registered): Promise<IconPackIcons> {
  if (!entry.loaded) {
    const { pack } = entry;
    entry.loaded =
      "icons" in pack
        ? Promise.resolve(pack.icons)
        : pack.loader().catch((e: unknown) => {
            entry.loaded = undefined; // let a later diagram try again
            throw new FacetryError(
              "invalid-options",
              `icon pack \`${pack.name}\` failed to load: ${e instanceof Error ? e.message : String(e)}`,
              { cause: e },
            );
          });
  }
  return entry.loaded;
}

/** Icon `name` of an Iconify set as SVG markup, following aliases. */
function fromIconify(set: IconifyJSON, pack: string, name: string): string | undefined {
  let icon = set.icons[name];
  let flipped = false;
  for (let hops = 0; !icon && hops < 8; hops++) {
    const alias = set.aliases?.[name];
    if (!alias) return undefined;
    flipped ||= !!(alias.rotate || alias.hFlip || alias.vFlip);
    name = alias.parent;
    icon = set.icons[name];
  }
  if (!icon) return undefined;
  if (flipped || icon.rotate || icon.hFlip || icon.vFlip) {
    throw new FacetryError(
      "invalid-options",
      `icon \`${pack}:${name}\` is rotated or flipped, which is not supported; use the icon it is made from`,
    );
  }
  const box = [
    icon.left ?? set.left ?? 0,
    icon.top ?? set.top ?? 0,
    icon.width ?? set.width ?? 16,
    icon.height ?? set.height ?? 16,
  ];
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box.join(" ")}">${icon.body}</svg>`;
}

/** Every icon name `@{ icon: ... }` in `source` names. */
export function iconNames(source: string): string[] {
  const out = new Set<string>();
  const meta = /@\{[^}\n]*\}/g;
  for (const m of source.matchAll(meta)) {
    const icon = /\bicon\s*:\s*(?:"([^"\n]*)"|([^\s,}]+))/.exec(m[0]);
    const name = icon?.[1] ?? icon?.[2];
    if (name) out.add(name.trim());
  }
  return [...out];
}

/**
 * The icons of registered packs that `source` names (`pack:icon`), as `DiagramOptions.icons`
 * takes them, leaving out those `given` already has. Loads the packs it needs. Names whose
 * prefix is no registered pack are left to the built-in icons.
 */
export async function packIcons(
  source: string,
  given: Record<string, IconSource>,
): Promise<Record<string, IconSource>> {
  const out: Record<string, IconSource> = {};
  for (const full of iconNames(source)) {
    if (full in given) continue;
    const colon = full.indexOf(":");
    if (colon <= 0) continue;
    const entry = packs.get(full.slice(0, colon));
    if (!entry) continue;
    const name = full.slice(colon + 1);
    const icons = await load(entry);
    const icon = isIconify(icons)
      ? fromIconify(icons, entry.pack.name, name)
      : (icons as Record<string, IconSource>)[name];
    if (icon === undefined) {
      throw new FacetryError("invalid-options", `icon \`${full}\` is not in the \`${entry.pack.name}\` icon pack`);
    }
    out[full] = icon;
  }
  return out;
}
