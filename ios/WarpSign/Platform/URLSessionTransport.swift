import Foundation

/// HTTP for warpsign-core's Developer Portal client. The Rust side keeps the
/// session's cookies and follows redirects itself, so this session does
/// neither: it sends exactly what it's given and returns exactly what came
/// back.
final class URLSessionTransport: NSObject, HttpTransport, URLSessionTaskDelegate, @unchecked Sendable {
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.urlCache = nil
        configuration.timeoutIntervalForRequest = 60
        return URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }()

    /// Called by Rust on a background thread; blocks until the response
    func send(request: HttpRequest) throws -> HttpResponse {
        guard let url = URL(string: request.url) else {
            throw TransportFailure.Failed(message: "invalid URL: \(request.url)")
        }
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = request.method
        for header in request.headers {
            urlRequest.addValue(header.value, forHTTPHeaderField: header.name)
        }
        if !request.body.isEmpty {
            urlRequest.httpBody = request.body
        }

        let outcome = Outcome()
        let done = DispatchSemaphore(value: 0)
        let task = session.dataTask(with: urlRequest) { data, response, error in
            defer { done.signal() }
            if let error {
                outcome.result = .failure(.Failed(message: error.localizedDescription))
                return
            }
            guard let response = response as? HTTPURLResponse else {
                outcome.result = .failure(.Failed(message: "not an HTTP response"))
                return
            }
            // Joined Set-Cookie headers are split again in Rust
            let headers = response.allHeaderFields.map { key, value in
                HttpHeader(name: "\(key)", value: "\(value)")
            }
            outcome.result = .success(HttpResponse(
                status: UInt16(clamping: response.statusCode),
                headers: headers,
                body: data ?? Data()
            ))
        }
        task.resume()
        done.wait()
        return try outcome.result.get()
    }

    /// Hand redirects back to Rust instead of following them
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest
    ) async -> URLRequest? {
        nil
    }

    /// Written once by the completion handler, read after the semaphore
    private final class Outcome: @unchecked Sendable {
        var result: Result<HttpResponse, TransportFailure> = .failure(.Failed(message: "no response"))
    }
}
