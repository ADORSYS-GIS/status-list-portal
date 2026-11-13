/**
 * Generated from the backend's OpenAPI document. Do not edit.
 *
 * Run `bun run test -u` to update it.
 */

export interface paths {
    "/api/v1/issuers": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** The signed-in user's issuers, deleted ones included, a page at a time. */
        get: operations["list_issuers"];
        put?: never;
        /**
         * Registers a sandbox issuer for the signed-in user.
         * @description It stays active for one sandbox lifetime. The account needs the current terms of use
         *     accepted and registrations left, and both the account and the sandbox need a free place.
         *     When registrations need an operator's approval, the request waits for it instead, and the
         *     Status List Server only sees the key then: a key it refuses, or one registered already,
         *     fails the approval rather than this request.
         */
        post: operations["create_issuer"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/issuers/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** One of the signed-in user's issuers. */
        get: operations["get_issuer"];
        put?: never;
        post?: never;
        /** Deletes one of the signed-in user's issuers: suspends it. */
        delete: operations["delete_issuer"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/issuers/{id}/renew": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Keeps one of the signed-in user's issuers active for another sandbox lifetime.
         * @description An expired issuer becomes active again, so it needs a free place, like a new one.
         */
        post: operations["renew_issuer"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/me": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** The signed-in user. */
        get: operations["me"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/me/terms": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /** Accepts the current terms of use on behalf of the signed-in user. */
        put: operations["accept_terms"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/registrations": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * The signed-in user's registrations, newest first. Pending and rejected ones count toward
         *     the issuers the account may register.
         */
        get: operations["list_registrations"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/registrations/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** Withdraws one of the signed-in user's registrations while it waits for approval. */
        delete: operations["withdraw_registration"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/session": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        /** Signs out of every browser at once. */
        delete: operations["sign_out"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/session/github": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Starts signing in: sends the browser to GitHub to approve the portal. */
        get: operations["begin"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/session/github/callback": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Finishes signing in when GitHub sends the browser back, then goes to the home page.
         * @description The browser is navigating, not fetching, so failures also go to the home page, which
         *     explains them.
         */
        get: operations["callback"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/terms": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** The current terms of use. */
        get: operations["get_terms"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/health/live": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Liveness probe: the process is up and answers requests. */
        get: operations["live"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/health/ready": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Readiness probe: a recent background check reached the database. */
        get: operations["ready"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        AcceptTermsRequest: {
            /**
             * @description Version of the terms the user read.
             * @example 2026-10-22
             */
            version: string;
        };
        /**
         * @description Stable reasons an API request can fail. The detail may change without changing the code.
         * @enum {string}
         */
        ApiErrorCode: "unauthorized" | "not_found" | "method_not_allowed" | "cross_origin_forbidden" | "terms_not_accepted" | "terms_changed" | "registration_limit_reached" | "active_issuer_limit_reached" | "sandbox_issuer_limit_reached" | "issuer_registration_in_progress" | "key_material_in_use" | "invalid_public_key" | "private_key_submitted" | "invalid_display_name" | "issuer_changed" | "issuer_not_renewable" | "registration_not_found" | "registration_being_decided" | "registration_already_decided" | "invalid_rejection_reason" | "invalid_json" | "invalid_request_body" | "body_read_failed" | "unsupported_media_type" | "payload_too_large" | "bad_gateway" | "service_unavailable" | "internal_error";
        ApiErrorDetail: {
            /** @description Stable machine-readable reason. Clients may branch on this value. */
            code: components["schemas"]["ApiErrorCode"];
            /** @description Human-readable description, safe to show to the user. */
            detail: string;
        };
        /** @description The body of every error response. */
        ApiErrorResponse: {
            /** @description What prevented the request from succeeding. Always exactly one entry. */
            errors: components["schemas"]["ApiErrorDetail"][];
        };
        IssuerKeyResponse: {
            alg: components["schemas"]["SigningAlgorithm"];
            /**
             * @description The `kid` in the header of the tokens the key signs.
             * @example QoRTRSkzlInq0eOmPqJwmZ5OS9pxkN_SIf5KgZP2F8M
             */
            kid: string;
            state: components["schemas"]["KeyState"];
        };
        IssuerListResponse: {
            issuers: components["schemas"]["IssuerResponse"][];
            /** @description Pass it as `cursor` to read the next page. `null` on the last page. */
            next_cursor: string | null;
        };
        IssuerResponse: {
            /**
             * @description The `aud` of the issuer's management tokens.
             * @example https://status.example.com
             */
            audience: string;
            /** Format: date-time */
            created_at: string | null;
            /** @example Acme Wallet Issuer */
            display_name: string | null;
            /**
             * Format: date-time
             * @description When the issuer stops being active, unless it is renewed.
             */
            expires_at: string | null;
            /**
             * @description The `iss` of the issuer's management tokens.
             * @example 6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00
             */
            id: string;
            /** @description Keys that may sign the issuer's management tokens, newest first. */
            keys: components["schemas"]["IssuerKeyResponse"][];
            /**
             * @description Where the issuer's management tokens go.
             * @example https://status.example.com
             */
            management_url: string;
            /** @description Only `active` issuers can write status lists. Deleted issuers are `suspended`. */
            state: components["schemas"]["IssuerState"];
        };
        /**
         * @description Only `active` issuers can write status lists.
         * @enum {string}
         */
        IssuerState: "active" | "suspended" | "expired";
        /** @enum {string} */
        KeyState: "active" | "reserved" | "revoked";
        MeResponse: {
            /**
             * @description Verified primary email on GitHub.
             * @example octocat@example.com
             */
            email: string;
            /**
             * @description GitHub login.
             * @example octocat
             */
            login: string;
            /**
             * @description Display name from the GitHub profile.
             * @example The Octocat
             */
            name: string | null;
            /** @description Whether the user accepted the current terms of use. Registering an issuer needs it. */
            terms_accepted: boolean;
        };
        /** @description A sandbox issuer to register. */
        NewIssuerRequest: {
            /** @description The algorithm the issuer signs its management tokens with. */
            alg: components["schemas"]["SigningAlgorithm"];
            /**
             * @description Shown in the portal and to the server's operators. 1 to 100 characters.
             * @example Acme Wallet Issuer
             */
            display_name: string;
            /** @description The public key that checks them, as a JSON Web Key. Private keys are refused. */
            public_key: Record<string, never>;
        };
        RegistrationListResponse: {
            registrations: components["schemas"]["RegistrationResponse"][];
        };
        RegistrationResponse: {
            alg: components["schemas"]["SigningAlgorithm"];
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            decided_at: string | null;
            /** @example Acme Wallet Issuer */
            display_name: string;
            /**
             * Format: int64
             * @example 42
             */
            id: number;
            /** @description The issuer the approval created. */
            issuer_id: string | null;
            /** @description Why an operator rejected the registration. */
            rejection_reason: string | null;
            state: components["schemas"]["RegistrationState"];
        };
        /** @enum {string} */
        RegistrationState: "pending" | "approved" | "rejected";
        /**
         * @description The JWS algorithm a key signs with.
         * @enum {string}
         */
        SigningAlgorithm: "ES256" | "ES384" | "EdDSA" | "PS256";
        TermsResponse: {
            /**
             * @description Where the current terms are published.
             * @example https://portal.example.com/terms
             */
            url: string;
            /**
             * @description Version of the current terms. Accepting them names it.
             * @example 2026-10-22
             */
            version: string;
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    list_issuers: {
        parameters: {
            query?: {
                /** @description The previous page's `next_cursor`. */
                cursor?: string;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description One page of the user's issuers. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["IssuerListResponse"];
                };
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The Status List Server did not complete the request. */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    create_issuer: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["NewIssuerRequest"];
            };
        };
        responses: {
            /** @description Registered. */
            201: {
                headers: {
                    location?: string;
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["IssuerResponse"];
                };
            };
            /** @description Waiting for an operator's approval. */
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RegistrationResponse"];
                };
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The current terms are not accepted, or the account registered as many issuers as it may. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The account has as many active issuers as allowed, or the key is registered already. */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The display name or the public key cannot be used. */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The Status List Server did not complete the request. */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The sandbox has no room for another issuer, or another one is being registered. */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    get_issuer: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description The issuer's id. */
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description The issuer. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["IssuerResponse"];
                };
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The user has no issuer with this id. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The Status List Server did not complete the request. */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    delete_issuer: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description The issuer's id. */
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Deleted, now or before. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The user has no issuer with this id. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The issuer changed while this request ran. */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The Status List Server did not complete the request. */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    renew_issuer: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description The issuer's id. */
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Renewed. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["IssuerResponse"];
                };
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The current terms are not accepted. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The user has no issuer with this id. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The issuer is deleted, the account has as many active issuers as allowed, or the issuer changed while this request ran. */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The Status List Server did not complete the request. */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The sandbox has no room for another issuer, or another one is being registered. */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    me: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description The signed-in user. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MeResponse"];
                };
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    accept_terms: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AcceptTermsRequest"];
            };
        };
        responses: {
            /** @description Accepted, now or before. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The terms changed since the user read them. */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    list_registrations: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description The user's registrations. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RegistrationListResponse"];
                };
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    withdraw_registration: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description The registration's id. */
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Withdrawn. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Not signed in. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The user has no registration with this id. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
            /** @description The registration was decided already, or is being decided. */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
    sign_out: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Signed out, or was not signed in. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    begin: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Redirects to GitHub. */
            303: {
                headers: {
                    location?: string;
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    callback: {
        parameters: {
            query?: {
                /** @description Code to trade for a token, when the user approved the portal. */
                code?: string;
                /** @description The `state` the sign-in started with. */
                state?: string;
                /** @description Why GitHub did not approve, for example `access_denied`. */
                error?: string;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Redirects to the home page. After a failure, its `sign_in_error` parameter says why: `denied`, `expired`, `no_verified_email` or `unavailable`. */
            303: {
                headers: {
                    location?: string;
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_terms: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description The current terms of use. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TermsResponse"];
                };
            };
        };
    };
    live: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description The server is running. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        /** @example ok */
                        status: string;
                    };
                };
            };
        };
    };
    ready: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description The server can handle requests. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        /** @example ok */
                        status: string;
                    };
                };
            };
            /** @description No recent successful database check is available. */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiErrorResponse"];
                };
            };
        };
    };
}
type FlattenedDeepRequired<T> = {
    [K in keyof T]-?: FlattenedDeepRequired<T[K] extends unknown[] | undefined | null ? Extract<T[K], unknown[]>[number] : T[K]>;
};
type ReadonlyArray<T> = [
    Exclude<T, undefined>
] extends [
    unknown[]
] ? Readonly<Exclude<T, undefined>> : Readonly<Exclude<T, undefined>[]>;
export const apiErrorCodeValues: ReadonlyArray<FlattenedDeepRequired<components>["schemas"]["ApiErrorCode"]> = ["unauthorized", "not_found", "method_not_allowed", "cross_origin_forbidden", "terms_not_accepted", "terms_changed", "registration_limit_reached", "active_issuer_limit_reached", "sandbox_issuer_limit_reached", "issuer_registration_in_progress", "key_material_in_use", "invalid_public_key", "private_key_submitted", "invalid_display_name", "issuer_changed", "issuer_not_renewable", "registration_not_found", "registration_being_decided", "registration_already_decided", "invalid_rejection_reason", "invalid_json", "invalid_request_body", "body_read_failed", "unsupported_media_type", "payload_too_large", "bad_gateway", "service_unavailable", "internal_error"];
export const issuerStateValues: ReadonlyArray<FlattenedDeepRequired<components>["schemas"]["IssuerState"]> = ["active", "suspended", "expired"];
export const keyStateValues: ReadonlyArray<FlattenedDeepRequired<components>["schemas"]["KeyState"]> = ["active", "reserved", "revoked"];
export const registrationStateValues: ReadonlyArray<FlattenedDeepRequired<components>["schemas"]["RegistrationState"]> = ["pending", "approved", "rejected"];
export const signingAlgorithmValues: ReadonlyArray<FlattenedDeepRequired<components>["schemas"]["SigningAlgorithm"]> = ["ES256", "ES384", "EdDSA", "PS256"];
