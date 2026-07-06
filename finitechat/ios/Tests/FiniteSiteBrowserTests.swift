import XCTest
@testable import FiniteChat

final class FiniteSiteBrowserTests: XCTestCase {
    func testNativeSessionAuthURLUsesOriginalSiteOrigin() throws {
        let url = try XCTUnwrap(URL(string: "https://draft.example.com:8443/posts/a%20b?view=full#top"))

        XCTAssertEqual(
            FiniteSiteNativeSessionPreflight.authURL(for: url)?.absoluteString,
            "https://draft.example.com:8443/_finite/auth/native-session"
        )
        XCTAssertEqual(
            FiniteSiteNativeSessionPreflight.returnPath(for: url),
            "/posts/a%20b?view=full#top"
        )
    }

    func testNativeSessionReturnPathDefaultsToRoot() throws {
        let url = try XCTUnwrap(URL(string: "https://draft.example.com"))

        XCTAssertEqual(FiniteSiteNativeSessionPreflight.returnPath(for: url), "/")
    }

    func testNativeSessionAuthURLRejectsNonWebURL() throws {
        let url = try XCTUnwrap(URL(string: "finitechat://room/abc"))

        XCTAssertFalse(FiniteSiteNativeSessionPreflight.canPreflight(url: url))
        XCTAssertNil(FiniteSiteNativeSessionPreflight.authURL(for: url))
    }

    func testNativeSessionNonceIsFiniteSitesTokenLike() {
        let nonce = FiniteSiteNativeSessionPreflight.makeNonce()
        let allowed = CharacterSet(
            charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.~"
        )

        XCTAssertGreaterThanOrEqual(nonce.utf8.count, 16)
        XCTAssertLessThanOrEqual(nonce.utf8.count, 128)
        XCTAssertNil(nonce.rangeOfCharacter(from: allowed.inverted))
    }
}
