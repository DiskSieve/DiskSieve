// Dev Junk kinds (ids mirror `JunkKind::as_str` in `disksieve-core/src/dev_junk.rs`).

interface KindInfo {
  /** Short label for badges and filters. */
  label: string;
  /** How the artifact comes back after deletion. */
  rebuild: string;
  /** `false` when deleting may lose something the tool does not recreate. */
  safe?: false;
}

const KINDS: Record<string, KindInfo> = {
  node_modules: { label: "node_modules", rebuild: "reinstall with `npm install` / `pnpm install` / `yarn`." },
  next_build: { label: ".next / .nuxt", rebuild: "rebuilt by the next `dev` or `build` run." },
  js_build: { label: "JS build", rebuild: "rebuilt by the project's build script." },
  js_tool_cache: { label: "JS tool cache", rebuild: "tool caches and coverage reports are recreated on the next run." },
  cargo_target: { label: "Rust target/", rebuild: "`cargo build` regenerates it (the next build is a full one)." },
  python_venv: { label: "Python venv", rebuild: "recreate with `python -m venv .venv` and reinstall the dependencies." },
  python_cache: { label: "Python cache", rebuild: "Python and its tools recreate caches automatically." },
  python_build: { label: "Python build", rebuild: "rebuilt by `pip install .`, `python -m build` or tox." },
  gradle_build: { label: "Gradle", rebuild: "rebuilt by the next Gradle build or sync." },
  maven_target: { label: "Maven target/", rebuild: "`mvn package` regenerates it." },
  dotnet_build: { label: ".NET bin/obj", rebuild: "`dotnet build` regenerates it." },
  php_vendor: { label: "PHP vendor/", rebuild: "`composer install` restores it." },
  ruby_bundle: { label: "Ruby bundle", rebuild: "`bundle install` restores it." },
  dart_build: { label: "Dart / Flutter", rebuild: "`flutter pub get` / `flutter build` regenerate it." },
  swift_build: { label: "Swift / Pods", rebuild: "`pod install` / `swift build` regenerate it." },
  cmake_build: { label: "CMake build", rebuild: "re-run the CMake configure and build." },
  elixir_build: { label: "Elixir", rebuild: "`mix deps.get` / `mix compile` regenerate it." },
  haskell_build: { label: "Haskell", rebuild: "`stack build` / `cabal build` regenerate it." },
  scala_build: { label: "Scala", rebuild: "rebuilt by the next sbt build or Metals import." },
  clojure_build: { label: "Clojure", rebuild: "rebuilt by the next build or REPL start." },
  zig_build: { label: "Zig", rebuild: "`zig build` regenerates it." },
  terraform_cache: { label: "Terraform", rebuild: "`terraform init` restores providers and modules." },
  unity_cache: { label: "Unity cache", rebuild: "Unity rebuilds it when the project is opened (this can take a while)." },
  unreal_cache: { label: "Unreal cache", rebuild: "Unreal regenerates caches, but `Saved` also holds autosaves and config — check it first.", safe: false },
  godot_cache: { label: "Godot cache", rebuild: "Godot reimports assets on the next open." },
};

/** Badge / filter label of a kind (falls back to the raw id). */
export const junkLabel = (kind: string) => KINDS[kind]?.label ?? kind;

/** Safety hint for an artifact, in the same "Safe to delete — …" form as the demo hints. */
export function junkHint(kind: string | null): string | null {
  const info = kind ? KINDS[kind] : undefined;
  if (!info) return null;
  return info.safe === false ? `Review first — ${info.rebuild}` : `Safe to delete — ${info.rebuild}`;
}
