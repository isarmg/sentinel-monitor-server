export function requireSameOriginMediaUrl(value: string, pageUrl: string): string {
  const page = new URL(pageUrl);
  const resolved = new URL(value, page);
  if (resolved.origin !== page.origin || resolved.username !== "" || resolved.password !== "") {
    throw new Error("Media URL must use the application origin");
  }
  return resolved.href;
}

export function isSameOriginMediaUrl(value: string, pageUrl: string): boolean {
  try {
    requireSameOriginMediaUrl(value, pageUrl);
    return true;
  } catch {
    return false;
  }
}

export function whepResourceUrl(location: string | null, requestUrl: string, pageUrl: string): string | null {
  if (location === null || location === "") return null;
  const request = requireSameOriginMediaUrl(requestUrl, pageUrl);
  const gatewayPath = new URL(request).pathname.startsWith("/media-webrtc/");
  const path = location.startsWith("/") && !location.startsWith("//") && gatewayPath
    ? `/media-webrtc${location}` : location;
  return requireSameOriginMediaUrl(path, request);
}
