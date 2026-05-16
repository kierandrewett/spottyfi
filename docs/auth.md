# Authentication

Spottyfi authenticates against Spotify twice over: an OAuth 2.0 PKCE flow for
the **Web API**, and a librespot session for **audio streaming**. They are not
the same token.

## OAuth 2.0 PKCE (Web API)

1. Generate a PKCE `code_verifier` / `code_challenge` pair.
2. Start a local HTTP server on `http://127.0.0.1:<random>/callback`.
3. Open the system browser to `accounts.spotify.com/authorize` with the client
   id, redirect URI, scopes and challenge.
4. Spotify redirects back to the local server with an authorization `code`.
5. Exchange `code` + `verifier` for an access token + refresh token.
6. Store the **refresh token** in the platform keyring under service
   `dev.drewett.spottyfi`, account = the Spotify user id.
7. A background task refreshes the access token before it expires.

### Scopes

```
user-read-private user-read-email user-read-playback-state
user-modify-playback-state user-read-currently-playing playlist-read-private
playlist-read-collaborative playlist-modify-private playlist-modify-public
user-library-read user-library-modify user-top-read user-read-recently-played
streaming app-remote-control user-follow-read user-follow-modify
```

### Client registration

The Spotify app is registered under the maintainer's own account. The redirect
URI registered there must match the one used at runtime **exactly**. Because the
callback port is randomised, register the loopback redirect that Spotify allows
for desktop apps and confirm the exact accepted form (see `questions.md`).

## librespot session (audio)

librespot needs its own credentials. The current path is
`Credentials::with_access_token`, but the librespot auth flow has moved twice
recently — **confirm against the latest librespot release before implementing**
(tracked in `questions.md`). The Web API PKCE token is *not* directly the token
librespot needs internally; investigate whether it can be reused or whether a
separate token exchange is required.

## Logout

Logout from the profile menu wipes the keyring entry and the on-disk cache, and
returns the app to the login screen.
