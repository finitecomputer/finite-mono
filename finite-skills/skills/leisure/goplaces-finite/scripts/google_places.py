#!/usr/bin/env python3
import argparse
import json
import os
import sys
from urllib.error import HTTPError
from urllib.request import Request, urlopen

API_BASE = "https://places.googleapis.com/v1"
SEARCH_FIELDS = ",".join([
    "places.id",
    "places.displayName",
    "places.formattedAddress",
    "places.googleMapsUri",
    "places.location",
    "places.rating",
    "places.userRatingCount",
    "places.businessStatus",
    "places.websiteUri",
    "places.nationalPhoneNumber",
    "places.priceLevel",
    "places.primaryTypeDisplayName",
    "places.regularOpeningHours.openNow",
    "places.regularOpeningHours.weekdayDescriptions",
])
DETAIL_FIELDS = ",".join([
    "id",
    "displayName",
    "formattedAddress",
    "googleMapsUri",
    "location",
    "rating",
    "userRatingCount",
    "businessStatus",
    "websiteUri",
    "nationalPhoneNumber",
    "priceLevel",
    "primaryTypeDisplayName",
    "regularOpeningHours.openNow",
    "regularOpeningHours.weekdayDescriptions",
])


def api_key() -> str:
    key = os.getenv("GOOGLE_PLACES_API_KEY")
    if not key:
        print("GOOGLE_PLACES_API_KEY is not set.", file=sys.stderr)
        raise SystemExit(2)
    return key


def request_json(method: str, url: str, *, body: dict | None = None, field_mask: str) -> dict:
    data = None
    headers = {
        "X-Goog-Api-Key": api_key(),
        "X-Goog-FieldMask": field_mask,
    }
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode("utf-8")
    req = Request(url, data=data, headers=headers, method=method)
    try:
        with urlopen(req) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except HTTPError as exc:
        payload = exc.read().decode("utf-8", errors="replace")
        print(payload, file=sys.stderr)
        raise SystemExit(exc.code)


def display_name(place: dict) -> str:
    name = place.get("displayName")
    if isinstance(name, dict):
        return name.get("text") or "<unknown>"
    return name or "<unknown>"


def primary_type(place: dict) -> str:
    value = place.get("primaryTypeDisplayName")
    if isinstance(value, dict):
        return value.get("text") or ""
    return value or ""


def print_search_results(payload: dict) -> None:
    places = payload.get("places", [])
    if not places:
        print("No places found.")
        return
    for i, place in enumerate(places, 1):
        print(f"{i}. {display_name(place)}")
        if place.get("formattedAddress"):
            print(f"   Address: {place['formattedAddress']}")
        ptype = primary_type(place)
        if ptype:
            print(f"   Type: {ptype}")
        if place.get("rating") is not None:
            count = place.get("userRatingCount")
            suffix = f" ({count} ratings)" if count is not None else ""
            print(f"   Rating: {place['rating']}{suffix}")
        if place.get("nationalPhoneNumber"):
            print(f"   Phone: {place['nationalPhoneNumber']}")
        if place.get("websiteUri"):
            print(f"   Website: {place['websiteUri']}")
        if place.get("googleMapsUri"):
            print(f"   Maps: {place['googleMapsUri']}")
        if place.get("id"):
            print(f"   Place ID: {place['id']}")
        opening = (place.get("regularOpeningHours") or {}).get("openNow")
        if opening is not None:
            print(f"   Open now: {opening}")
        weekday = (place.get("regularOpeningHours") or {}).get("weekdayDescriptions") or []
        if weekday:
            print("   Hours:")
            for line in weekday:
                print(f"     {line}")
        print()


def print_details(place: dict) -> None:
    print_search_results({"places": [place]})


def main() -> int:
    parser = argparse.ArgumentParser(description="Google Places API helper")
    sub = parser.add_subparsers(dest="command", required=True)

    p_search = sub.add_parser("search", help="Text search for places")
    p_search.add_argument("--query", required=True)
    p_search.add_argument("--max-results", type=int, default=5)
    p_search.add_argument("--language-code")
    p_search.add_argument("--region-code")
    p_search.add_argument("--lat", type=float)
    p_search.add_argument("--lon", type=float)
    p_search.add_argument("--radius-meters", type=float)
    p_search.add_argument("--json", action="store_true")

    p_details = sub.add_parser("details", help="Get place details by place ID")
    p_details.add_argument("--place-id", required=True)
    p_details.add_argument("--language-code")
    p_details.add_argument("--region-code")
    p_details.add_argument("--json", action="store_true")

    args = parser.parse_args()

    if args.command == "search":
        body = {
            "textQuery": args.query,
            "pageSize": args.max_results,
        }
        if args.language_code:
            body["languageCode"] = args.language_code
        if args.region_code:
            body["regionCode"] = args.region_code
        if any(v is not None for v in (args.lat, args.lon, args.radius_meters)):
            if None in (args.lat, args.lon, args.radius_meters):
                print("--lat, --lon, and --radius-meters must be provided together.", file=sys.stderr)
                return 2
            body["locationBias"] = {
                "circle": {
                    "center": {"latitude": args.lat, "longitude": args.lon},
                    "radius": args.radius_meters,
                }
            }
        payload = request_json(
            "POST",
            f"{API_BASE}/places:searchText",
            body=body,
            field_mask=SEARCH_FIELDS,
        )
        if args.json:
            print(json.dumps(payload, indent=2))
        else:
            print_search_results(payload)
        return 0

    if args.command == "details":
        url = f"{API_BASE}/places/{args.place_id}"
        query = []
        if args.language_code:
            query.append(f"languageCode={args.language_code}")
        if args.region_code:
            query.append(f"regionCode={args.region_code}")
        if query:
            url = url + "?" + "&".join(query)
        payload = request_json("GET", url, field_mask=DETAIL_FIELDS)
        if args.json:
            print(json.dumps(payload, indent=2))
        else:
            print_details(payload)
        return 0

    return 2


if __name__ == "__main__":
    raise SystemExit(main())
