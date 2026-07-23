import { randomBytes } from "node:crypto";

import { initializeApp } from "firebase-admin/app";
import { FieldValue, Timestamp, getFirestore } from "firebase-admin/firestore";
import { getStorage } from "firebase-admin/storage";
import { onRequest } from "firebase-functions/v2/https";
import { onSchedule } from "firebase-functions/v2/scheduler";
import type { Response } from "express";

initializeApp();

const REGION = "europe-west1";
const COLLECTION = "profileShares";
const SHARE_LIFETIME_MS = 24 * 60 * 60 * 1000;
const SIGNED_URL_LIFETIME_MS = 15 * 60 * 1000;
const MAX_SHARE_BYTES = 1024 * 1024 * 1024;
const PROFILE_CONTENT_TYPE = "application/vnd.uniloader.profile+zip";
const CODE_ALPHABET = "23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const CODE_LENGTH = 16;

interface ShareDocument {
  appVersion: string;
  createdAt: Timestamp;
  expiresAt: Timestamp;
  objectName: string;
  profileName: string;
  sha256: string;
  sizeBytes: number;
  status: "pending" | "ready";
}

class RequestValidationError extends Error {}

export const profileShareApi = onRequest(
  {
    region: REGION,
    timeoutSeconds: 540,
    memory: "512MiB",
    maxInstances: 10
  },
  async (request, response) => {
    applyResponseHeaders(response);
    if (request.method === "OPTIONS") {
      response.status(204).send("");
      return;
    }

    try {
      const path = request.path.replace(/\/+$/, "") || "/";
      if (request.method === "POST" && path === "/v1/shares") {
        await createShare(request.body, response);
        return;
      }

      const completeMatch = path.match(/^\/v1\/shares\/([^/]+)\/complete$/);
      if (request.method === "POST" && completeMatch) {
        await completeShare(completeMatch[1], request.body, response);
        return;
      }

      const resolveMatch = path.match(/^\/v1\/shares\/([^/]+)$/);
      if (request.method === "GET" && resolveMatch) {
        await resolveShare(resolveMatch[1], response);
        return;
      }

      sendError(response, 404, "not-found", "Profile share endpoint not found.");
    } catch (error) {
      if (error instanceof RequestValidationError) {
        sendError(response, 400, "invalid-request", error.message);
        return;
      }
      console.error("Unhandled profile share error", error);
      sendError(response, 500, "internal", "The profile share service could not complete the request.");
    }
  }
);

export const cleanupExpiredProfileShares = onSchedule(
  {
    schedule: "every 60 minutes",
    region: REGION,
    timeoutSeconds: 540,
    memory: "512MiB"
  },
  async () => {
    const database = getFirestore();
    const bucket = getStorage().bucket();
    let removed = 0;

    for (let pass = 0; pass < 10; pass += 1) {
      const snapshot = await database
        .collection(COLLECTION)
        .where("expiresAt", "<=", Timestamp.now())
        .limit(200)
        .get();
      if (snapshot.empty) {
        break;
      }

      await Promise.all(
        snapshot.docs.map(async (document) => {
          const share = document.data() as ShareDocument;
          await bucket.file(share.objectName).delete({ ignoreNotFound: true });
          await document.ref.delete();
          removed += 1;
        })
      );
    }

    console.log(`Removed ${removed} expired UniLoader profile share(s).`);
  }
);

async function createShare(body: unknown, response: Response): Promise<void> {
  const payload = asRecord(body);
  const profileName = requiredString(payload.profileName, "profileName", 120);
  const appVersion = requiredString(payload.appVersion, "appVersion", 40);
  const sha256 = requiredSha256(payload.sha256);
  const sizeBytes = requiredInteger(payload.sizeBytes, "sizeBytes");
  if (sizeBytes <= 0 || sizeBytes > MAX_SHARE_BYTES) {
    sendError(response, 413, "share-too-large", "The profile share is outside the 1 GB size limit.");
    return;
  }

  const database = getFirestore();
  const bucket = getStorage().bucket();
  const now = Date.now();
  const expiresAt = Timestamp.fromMillis(now + SHARE_LIFETIME_MS);
  let code = "";
  let objectName = "";

  for (let attempt = 0; attempt < 5; attempt += 1) {
    code = createCode();
    objectName = `profile-shares/${code}/${randomToken(18)}.uniloader-profile`;
    const share: ShareDocument = {
      appVersion,
      createdAt: Timestamp.fromMillis(now),
      expiresAt,
      objectName,
      profileName,
      sha256,
      sizeBytes,
      status: "pending"
    };
    try {
      await database.collection(COLLECTION).doc(code).create(share);
      break;
    } catch (error) {
      const codeValue = (error as { code?: number | string }).code;
      if (codeValue !== 6 && codeValue !== "already-exists") {
        throw error;
      }
      code = "";
    }
  }

  if (!code) {
    sendError(response, 503, "key-generation-failed", "A unique share key could not be generated.");
    return;
  }

  try {
    const uploadHeaders = {
      "content-type": PROFILE_CONTENT_TYPE,
      "x-goog-meta-sha256": sha256,
      "x-goog-meta-size-bytes": String(sizeBytes)
    };
    const [uploadUrl] = await bucket.file(objectName).getSignedUrl({
      version: "v4",
      action: "write",
      expires: now + SIGNED_URL_LIFETIME_MS,
      contentType: PROFILE_CONTENT_TYPE,
      extensionHeaders: {
        "x-goog-meta-sha256": sha256,
        "x-goog-meta-size-bytes": String(sizeBytes)
      }
    });
    response.status(201).json({
      code: formatCode(code),
      expiresAt: expiresAt.toDate().toISOString(),
      uploadUrl,
      uploadHeaders
    });
  } catch (error) {
    await database.collection(COLLECTION).doc(code).delete().catch(() => undefined);
    throw error;
  }
}

async function completeShare(rawCode: string, body: unknown, response: Response): Promise<void> {
  const code = normalizeCode(rawCode);
  if (!code) {
    sendError(response, 400, "invalid-key", "The UniLoader key is invalid.");
    return;
  }
  const payload = asRecord(body);
  const sha256 = requiredSha256(payload.sha256);
  const sizeBytes = requiredInteger(payload.sizeBytes, "sizeBytes");
  const database = getFirestore();
  const reference = database.collection(COLLECTION).doc(code);
  const snapshot = await reference.get();
  if (!snapshot.exists) {
    sendError(response, 404, "not-found", "This UniLoader key was not found.");
    return;
  }
  const share = snapshot.data() as ShareDocument;
  if (isExpired(share)) {
    await deleteShare(reference, share);
    sendError(response, 410, "expired", "This UniLoader key has expired.");
    return;
  }
  if (share.sha256 !== sha256 || share.sizeBytes !== sizeBytes) {
    await deleteShare(reference, share);
    sendError(response, 422, "metadata-mismatch", "The uploaded profile did not match its signed metadata.");
    return;
  }

  const file = getStorage().bucket().file(share.objectName);
  const [metadata] = await file.getMetadata();
  const uploadedSize = Number(metadata.size);
  const uploadedHash = metadata.metadata?.sha256;
  const uploadedDeclaredSize = Number(metadata.metadata?.["size-bytes"]);
  if (
    uploadedSize !== share.sizeBytes ||
    uploadedDeclaredSize !== share.sizeBytes ||
    uploadedHash !== share.sha256
  ) {
    await deleteShare(reference, share);
    sendError(response, 422, "upload-mismatch", "The uploaded profile failed validation.");
    return;
  }

  await reference.update({
    status: "ready",
    completedAt: FieldValue.serverTimestamp()
  });
  response.status(200).json({
    code: formatCode(code),
    expiresAt: share.expiresAt.toDate().toISOString()
  });
}

async function resolveShare(rawCode: string, response: Response): Promise<void> {
  const code = normalizeCode(rawCode);
  if (!code) {
    sendError(response, 400, "invalid-key", "The UniLoader key is invalid.");
    return;
  }
  const database = getFirestore();
  const reference = database.collection(COLLECTION).doc(code);
  const snapshot = await reference.get();
  if (!snapshot.exists) {
    sendError(response, 404, "not-found", "This UniLoader key was not found.");
    return;
  }
  const share = snapshot.data() as ShareDocument;
  if (isExpired(share)) {
    await deleteShare(reference, share);
    sendError(response, 410, "expired", "This UniLoader key has expired.");
    return;
  }
  if (share.status !== "ready") {
    sendError(response, 409, "not-ready", "This profile share has not finished uploading.");
    return;
  }

  const [downloadUrl] = await getStorage().bucket().file(share.objectName).getSignedUrl({
    version: "v4",
    action: "read",
    expires: Date.now() + SIGNED_URL_LIFETIME_MS,
    responseDisposition: "attachment; filename=\"UniLoader-Profile.uniloader-profile\"",
    responseType: PROFILE_CONTENT_TYPE
  });
  response.status(200).json({
    code: formatCode(code),
    expiresAt: share.expiresAt.toDate().toISOString(),
    downloadUrl,
    sizeBytes: share.sizeBytes,
    sha256: share.sha256
  });
}

function asRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new RequestValidationError("Request body must be a JSON object.");
  }
  return value as Record<string, unknown>;
}

function requiredString(value: unknown, field: string, maxLength: number): string {
  if (typeof value !== "string" || !value.trim() || value.trim().length > maxLength) {
    throw new RequestValidationError(`${field} is invalid.`);
  }
  return value.trim();
}

function requiredInteger(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    throw new RequestValidationError(`${field} is invalid.`);
  }
  return value;
}

function requiredSha256(value: unknown): string {
  const normalized = requiredString(value, "sha256", 64).toLowerCase();
  if (!/^[a-f0-9]{64}$/.test(normalized)) {
    throw new RequestValidationError("sha256 is invalid.");
  }
  return normalized;
}

function createCode(): string {
  const bytes = randomBytes(CODE_LENGTH);
  return Array.from(bytes, (byte) => CODE_ALPHABET[byte % CODE_ALPHABET.length]).join("");
}

function randomToken(length: number): string {
  return randomBytes(length).toString("hex");
}

function normalizeCode(value: string): string | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(value);
  } catch {
    return null;
  }
  const normalized = decoded.replace(/[\s-]/g, "").toUpperCase();
  return normalized.length === CODE_LENGTH &&
    Array.from(normalized).every((character) => CODE_ALPHABET.includes(character))
    ? normalized
    : null;
}

function formatCode(code: string): string {
  return code.match(/.{1,4}/g)?.join("-") ?? code;
}

function isExpired(share: ShareDocument): boolean {
  return Date.now() >= share.expiresAt.toMillis();
}

async function deleteShare(
  reference: FirebaseFirestore.DocumentReference,
  share: ShareDocument
): Promise<void> {
  await Promise.all([
    getStorage().bucket().file(share.objectName).delete({ ignoreNotFound: true }),
    reference.delete()
  ]);
}

function applyResponseHeaders(response: Response): void {
  response.set("Cache-Control", "no-store");
  response.set("Access-Control-Allow-Origin", "*");
  response.set("Access-Control-Allow-Headers", "content-type");
  response.set("Access-Control-Allow-Methods", "GET,POST,OPTIONS");
}

function sendError(
  response: Response,
  status: number,
  code: string,
  message: string
): void {
  response.status(status).json({
    error: {
      code,
      message
    }
  });
}
