export const env = {
  // Cognito Hosted UI domain (host only, no scheme), e.g. dev-auth-k8s.sancode.dev
  cognitoDomain: import.meta.env.VITE_COGNITO_DOMAIN as string,
  cognitoClientId: import.meta.env.VITE_COGNITO_CLIENT_ID as string,
} as const;
