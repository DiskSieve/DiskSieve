// Dev Junk kinds (ids mirror `JunkKind::as_str` in `disksieve-core/src/dev_junk.rs`).
// How each kind is rebuilt lives in Rust (`JunkKind::rebuild_hint`) and reaches the UI
// through the Safety Advisor (`getAdvice`).

/** Short labels for badges and filters. */
const LABELS: Record<string, string> = {
  node_modules: "node_modules",
  next_build: ".next / .nuxt",
  js_build: "JS build",
  js_tool_cache: "JS tool cache",
  cargo_target: "Rust target/",
  python_venv: "Python venv",
  python_cache: "Python cache",
  python_build: "Python build",
  gradle_build: "Gradle",
  maven_target: "Maven target/",
  dotnet_build: ".NET bin/obj",
  php_vendor: "PHP vendor/",
  ruby_bundle: "Ruby bundle",
  dart_build: "Dart / Flutter",
  swift_build: "Swift / Pods",
  cmake_build: "CMake build",
  elixir_build: "Elixir",
  haskell_build: "Haskell",
  scala_build: "Scala",
  clojure_build: "Clojure",
  zig_build: "Zig",
  terraform_cache: "Terraform",
  unity_cache: "Unity cache",
  unreal_cache: "Unreal cache",
  godot_cache: "Godot cache",
};

/** Badge / filter label of a kind (falls back to the raw id). */
export const junkLabel = (kind: string) => LABELS[kind] ?? kind;
