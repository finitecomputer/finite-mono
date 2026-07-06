import Foundation
import XCTest
@testable import FiniteChat

final class RemoteImageCacheTests: XCTestCase {
    func testRemoteImageDiskCachePersistsDataForRemoteURL() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("FiniteChatTests", isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let cache = RemoteImageDiskCache(directory: directory)
        let url = try XCTUnwrap(URL(string: "https://example.com/avatar.png?size=128"))
        let data = Data([0x89, 0x50, 0x4e, 0x47])

        cache.setData(data, for: url)

        XCTAssertEqual(cache.data(for: url), data)
    }

    func testRemoteImageDiskCacheIgnoresFileURLs() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("FiniteChatTests", isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let cache = RemoteImageDiskCache(directory: directory)
        let url = URL(fileURLWithPath: "/tmp/avatar.png")

        cache.setData(Data([1, 2, 3]), for: url)

        XCTAssertNil(cache.data(for: url))
    }
}
