import { DependencySpec, InstallPlan, ThunderstoreManifest } from "../../shared/contracts";
import { mergeDependencies, parseThunderstoreDependency } from "./knownDependencies";

export function attachManifestDependencies(
  plan: InstallPlan,
  manifest?: ThunderstoreManifest
): InstallPlan {
  const manifestDependencies: DependencySpec[] =
    manifest?.dependencies?.map(parseThunderstoreDependency) ?? [];

  return {
    ...plan,
    dependencies: mergeDependencies([...plan.dependencies, ...manifestDependencies])
  };
}
