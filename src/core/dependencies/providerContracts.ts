import { DependencySpec } from "../../shared/contracts";

export interface DependencyDownload {
  dependency: DependencySpec;
  archivePath: string;
  sourceUrl?: string;
}

export interface DependencyProviderClient {
  id: DependencySpec["provider"];
  canResolve(dependency: DependencySpec): boolean;
  resolve(dependency: DependencySpec): Promise<DependencySpec>;
  download?(dependency: DependencySpec, destinationDir: string): Promise<DependencyDownload>;
}

export class ManualDependencyProvider implements DependencyProviderClient {
  id: DependencySpec["provider"] = "manual";

  canResolve(): boolean {
    return true;
  }

  async resolve(dependency: DependencySpec): Promise<DependencySpec> {
    return {
      ...dependency,
      status: dependency.status === "already-installed" ? dependency.status : "manual",
      notes: dependency.notes ?? "Manual install or provider integration is required."
    };
  }
}
