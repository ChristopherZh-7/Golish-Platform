/**
 * Compat re-export — actual implementation lives at `./models/index`.
 *
 * Surface narrowed to symbols that are imported through this wrapper path
 * (M2.3 cleanup — previously `export *` re-exported the entire module).
 */

export {
  formatModelName,
  getProviderGroup,
  getProviderGroupNested,
  type ModelEntry,
  PROVIDER_GROUPS,
  PROVIDER_GROUPS_NESTED,
  type ProviderGroupNested,
} from "./models/index";
