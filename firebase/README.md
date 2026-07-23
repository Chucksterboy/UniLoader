# UniLoader Profile Share Service

This Firebase service stores self-contained UniLoader profile bundles behind short,
high-entropy keys. A key expires exactly 24 hours after creation. API requests reject
expired keys immediately; an hourly cleanup function removes the private Storage object
and Firestore metadata afterward.

## Firebase setup

1. Create a Firebase project on the Blaze plan, then enable Firestore and Cloud Storage.
2. Copy `.firebaserc.example` to `.firebaserc` and replace the project ID.
3. Install the function dependencies:

   ```powershell
   npm --prefix firebase/functions install
   ```

4. Sign in and deploy:

   ```powershell
   firebase login
   firebase deploy --only functions,firestore:rules,storage
   ```

5. Copy the deployed `profileShareApi` URL.
6. Add a GitHub repository variable named `UNILOADER_PROFILE_SHARE_API_URL` containing
   that URL. Release builds compile the service URL into UniLoader.

For local development, set the variable before running Tauri:

```powershell
$env:UNILOADER_PROFILE_SHARE_API_URL = "http://127.0.0.1:5001/PROJECT/europe-west1/profileShareApi"
pnpm dev
```

The Storage and Firestore rules deny direct client access. UniLoader receives short-lived
signed URLs from the function, verifies the declared size and SHA-256 before upload, and
verifies both again after download.

Only generate keys for mod files whose licenses permit redistribution. Firebase billing
and budget alerts should be configured before sharing the service beyond a small trusted
group.
