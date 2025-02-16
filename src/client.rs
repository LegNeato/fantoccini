use crate::elements::{Element, Form};
use crate::session::{Cmd, Session, Task};
use crate::{error, Locator};
use hyper::{client::connect, Method};
use serde_json::Value as Json;
use std::convert::TryFrom;
use std::future::Future;
use tokio::sync::{mpsc, oneshot};
use webdriver::command::{
    NewWindowParameters, SwitchToFrameParameters, SwitchToWindowParameters, WebDriverCommand,
};
use webdriver::common::{FrameId, ELEMENT_KEY};

// Used only under `native-tls`
#[cfg_attr(not(feature = "native-tls"), allow(unused_imports))]
use crate::ClientBuilder;

/// A WebDriver client tied to a single browser
/// [session](https://www.w3.org/TR/webdriver1/#sessions).
///
/// Use [`ClientBuilder`](crate::ClientBuilder) to create a new session.
///
/// Note that most callers should explicitly call `Client::close`, and wait for the returned
/// future before exiting. Not doing so may result in the WebDriver session not being cleanly
/// closed, which is particularly important for some drivers, such as geckodriver, where
/// multiple simulatenous sessions are not supported. If `close` is not explicitly called, a
/// session close request will be spawned on the given `handle` when the last instance of this
/// `Client` is dropped.
#[derive(Clone, Debug)]
pub struct Client {
    pub(crate) tx: mpsc::UnboundedSender<Task>,
    pub(crate) is_legacy: bool,
}

impl Client {
    /// Connect to the WebDriver host running the given address.
    ///
    /// This connects using a platform-native TLS library, and is only available with the
    /// `native-tls` feature. To customize, use [`ClientBuilder`] instead.
    #[cfg(feature = "native-tls")]
    #[cfg_attr(docsrs, doc(cfg(feature = "native-tls")))]
    #[deprecated(since = "0.17.1", note = "Prefer ClientBuilder::native")]
    pub async fn new(webdriver: &str) -> Result<Self, error::NewSessionError> {
        ClientBuilder::native().connect(webdriver).await
    }

    /// Connect to the WebDriver host running the given address.
    ///
    /// The provided `connector` is used to establish the connection to the WebDriver host, and
    /// should generally be one that supports HTTPS, as that is commonly required by WebDriver
    /// implementations.
    ///
    /// Calls `with_capabilities_and_connector` with an empty capabilities list.
    pub(crate) async fn new_with_connector<C>(
        webdriver: &str,
        connector: C,
    ) -> Result<Self, error::NewSessionError>
    where
        C: connect::Connect + Unpin + 'static + Clone + Send + Sync,
    {
        Self::with_capabilities_and_connector(
            webdriver,
            &webdriver::capabilities::Capabilities::new(),
            connector,
        )
        .await
    }

    /// Connect to the WebDriver host running the given address.
    ///
    /// Prefer using [`ClientBuilder`](crate::ClientBuilder) over calling this method directly.
    ///
    /// The given capabilities will be requested in `alwaysMatch` or `desiredCapabilities`
    /// depending on the protocol version supported by the server.
    ///
    /// Returns a future that resolves to a handle for issuing additional WebDriver tasks.
    pub async fn with_capabilities_and_connector<C>(
        webdriver: &str,
        cap: &webdriver::capabilities::Capabilities,
        connector: C,
    ) -> Result<Self, error::NewSessionError>
    where
        C: connect::Connect + Unpin + 'static + Clone + Send + Sync,
    {
        Session::with_capabilities_and_connector(webdriver, cap, connector).await
    }

    /// Get the unique session ID assigned by the WebDriver server to this client.
    pub async fn session_id(&mut self) -> Result<Option<String>, error::CmdError> {
        match self.issue(Cmd::GetSessionId).await? {
            Json::String(s) => Ok(Some(s)),
            Json::Null => Ok(None),
            v => unreachable!("response to GetSessionId was not a string: {:?}", v),
        }
    }

    /// Set the User Agent string to use for all subsequent requests.
    pub async fn set_ua<S: Into<String>>(&mut self, ua: S) -> Result<(), error::CmdError> {
        self.issue(Cmd::SetUa(ua.into())).await?;
        Ok(())
    }

    /// Get the current User Agent string.
    pub async fn get_ua(&mut self) -> Result<Option<String>, error::CmdError> {
        match self.issue(Cmd::GetUa).await? {
            Json::String(s) => Ok(Some(s)),
            Json::Null => Ok(None),
            v => unreachable!("response to GetSessionId was not a string: {:?}", v),
        }
    }

    /// Terminate the WebDriver session.
    ///
    /// Normally, a shutdown of the WebDriver connection will be initiated when the last clone of a
    /// `Client` is dropped. Specifically, the shutdown request will be issued using the tokio
    /// `Handle` given when creating this `Client`. This in turn means that any errors will be
    /// dropped.
    ///
    /// This function is safe to call multiple times, but once it has been called on one instance
    /// of a `Client`, all requests to other instances of that `Client` will fail.
    ///
    /// This function may be useful in conjunction with `raw_client_for`, as it allows you to close
    /// the automated browser window while doing e.g., a large download.
    pub async fn close(&mut self) -> Result<(), error::CmdError> {
        self.issue(Cmd::Shutdown).await?;
        Ok(())
    }

    /// Mark this client's session as persistent.
    ///
    /// After all instances of a `Client` have been dropped, we normally shut down the WebDriver
    /// session, which also closes the associated browser window or tab. By calling this method,
    /// the shutdown command will _not_ be sent to this client's session, meaning its window or tab
    /// will remain open.
    ///
    /// Note that an explicit call to [`Client::close`] will still terminate the session.
    ///
    /// This function is safe to call multiple times.
    pub async fn persist(&mut self) -> Result<(), error::CmdError> {
        self.issue(Cmd::Persist).await?;
        Ok(())
    }
}

// NOTE: new impl block to keep related methods together.

/// [Navigation](https://www.w3.org/TR/webdriver1/#navigation)
impl Client {
    /// Navigate directly to the given URL.
    ///
    /// See [9.1 Navigate To](https://www.w3.org/TR/webdriver1/#dfn-navigate-to) of the WebDriver
    /// standard.
    #[cfg_attr(docsrs, doc(alias = "Navigate To"))]
    pub async fn goto(&mut self, url: &str) -> Result<(), error::CmdError> {
        let url = url.to_owned();
        let base = self.current_url_().await?;
        let url = base.join(&url)?;
        self.issue(WebDriverCommand::Get(webdriver::command::GetParameters {
            url: url.into_string(),
        }))
        .await?;
        Ok(())
    }

    /// Retrieve the currently active URL for this session.
    ///
    /// See [9.2 Get Current URL](https://www.w3.org/TR/webdriver1/#dfn-get-current-url) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Current URL"))]
    pub async fn current_url(&mut self) -> Result<url::Url, error::CmdError> {
        self.current_url_().await
    }

    pub(crate) async fn current_url_(&mut self) -> Result<url::Url, error::CmdError> {
        let url = self.issue(WebDriverCommand::GetCurrentUrl).await?;
        if let Some(url) = url.as_str() {
            let url = if url.is_empty() { "about:blank" } else { url };
            Ok(url.parse()?)
        } else {
            Err(error::CmdError::NotW3C(url))
        }
    }

    /// Go back to the previous page.
    ///
    /// See [9.3 Back](https://www.w3.org/TR/webdriver1/#dfn-back) of the WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Back"))]
    pub async fn back(&mut self) -> Result<(), error::CmdError> {
        self.issue(WebDriverCommand::GoBack).await?;
        Ok(())
    }

    /// Refresh the current previous page.
    ///
    /// See [9.5 Refresh](https://www.w3.org/TR/webdriver1/#dfn-refresh) of the WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Refresh"))]
    pub async fn refresh(&mut self) -> Result<(), error::CmdError> {
        self.issue(WebDriverCommand::Refresh).await?;
        Ok(())
    }
}

/// [Command Contexts](https://www.w3.org/TR/webdriver1/#command-contexts)
impl Client {
    /// Gets the current window handle.
    ///
    /// See [10.1 Get Window Handle](https://www.w3.org/TR/webdriver1/#get-window-handle) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Window Handle"))]
    pub async fn window(&mut self) -> Result<webdriver::common::WebWindow, error::CmdError> {
        let res = self.issue(WebDriverCommand::GetWindowHandle).await?;
        match res {
            Json::String(x) => Ok(webdriver::common::WebWindow(x)),
            v => Err(error::CmdError::NotW3C(v)),
        }
    }

    /// Closes the current window.
    ///
    /// Will close the session if no other windows exist.
    ///
    /// Closing a window will not switch the client to one of the remaining windows.
    /// The switching must be done by calling `switch_to_window` using a still live window
    /// after the current window has been closed.
    ///
    /// See [10.2 Close Window](https://www.w3.org/TR/webdriver1/#close-window) of the WebDriver
    /// standard.
    #[cfg_attr(docsrs, doc(alias = "Close Window"))]
    pub async fn close_window(&mut self) -> Result<(), error::CmdError> {
        let _res = self.issue(WebDriverCommand::CloseWindow).await?;
        Ok(())
    }

    /// Switches to the chosen window.
    ///
    /// See [10.3 Switch To Window](https://www.w3.org/TR/webdriver1/#switch-to-window) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Switch To Window"))]
    pub async fn switch_to_window(
        &mut self,
        window: webdriver::common::WebWindow,
    ) -> Result<(), error::CmdError> {
        let params = SwitchToWindowParameters { handle: window.0 };
        let _res = self.issue(WebDriverCommand::SwitchToWindow(params)).await?;
        Ok(())
    }

    /// Gets a list of all active windows (and tabs)
    ///
    /// See [10.4 Get Window Handles](https://www.w3.org/TR/webdriver1/#get-window-handles) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Window Handles"))]
    pub async fn windows(&mut self) -> Result<Vec<webdriver::common::WebWindow>, error::CmdError> {
        let res = self.issue(WebDriverCommand::GetWindowHandles).await?;
        match res {
            Json::Array(handles) => handles
                .into_iter()
                .map(|handle| match handle {
                    Json::String(x) => Ok(webdriver::common::WebWindow(x)),
                    v => Err(error::CmdError::NotW3C(v)),
                })
                .collect::<Result<Vec<_>, _>>(),
            v => Err(error::CmdError::NotW3C(v)),
        }
    }

    /// Creates a new window. If `is_tab` is `true`, then a tab will be created instead.
    ///
    /// Windows are treated the same as tabs by the WebDriver protocol. The functions `new_window`,
    /// `switch_to_window`, `close_window`, `window` and `windows` all operate on both tabs and
    /// windows.
    ///
    /// This operation is only in the editor's draft of the next iteration of the WebDriver
    /// protocol, and may thus not be supported by all WebDriver implementations. For example, if
    /// you're using `geckodriver`, you will need `geckodriver > 0.24` and `firefox > 66` to use
    /// this feature.
    ///
    /// See [11.5 New Window](https://w3c.github.io/webdriver/#dfn-new-window) of the editor's
    /// draft standard.
    #[cfg_attr(docsrs, doc(alias = "New Window"))]
    pub async fn new_window(
        &mut self,
        as_tab: bool,
    ) -> Result<webdriver::response::NewWindowResponse, error::CmdError> {
        let type_hint = if as_tab { "tab" } else { "window" }.to_string();
        let type_hint = Some(type_hint);
        let params = NewWindowParameters { type_hint };
        match self.issue(WebDriverCommand::NewWindow(params)).await? {
            Json::Object(mut obj) => {
                let handle = match obj
                    .remove("handle")
                    .and_then(|x| x.as_str().map(String::from))
                {
                    Some(handle) => handle,
                    None => return Err(error::CmdError::NotW3C(Json::Object(obj))),
                };

                let typ = match obj
                    .remove("type")
                    .and_then(|x| x.as_str().map(String::from))
                {
                    Some(typ) => typ,
                    None => return Err(error::CmdError::NotW3C(Json::Object(obj))),
                };

                Ok(webdriver::response::NewWindowResponse { handle, typ })
            }
            v => Err(error::CmdError::NotW3C(v)),
        }
    }

    /// Switches to the frame specified at the index.
    ///
    /// See [10.5 Switch To Frame](https://www.w3.org/TR/webdriver1/#switch-to-frame) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Switch To Frame"))]
    pub async fn enter_frame(mut self, index: Option<u16>) -> Result<Client, error::CmdError> {
        let params = SwitchToFrameParameters {
            id: index.map(FrameId::Short),
        };
        self.issue(WebDriverCommand::SwitchToFrame(params)).await?;
        Ok(self)
    }

    /// Switches to the parent of the frame the client is currently contained within.
    ///
    /// See [10.6 Switch To Parent Frame](https://www.w3.org/TR/webdriver1/#switch-to-parent-frame)
    /// of the WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Switch To Parent Frame"))]
    pub async fn enter_parent_frame(mut self) -> Result<Client, error::CmdError> {
        self.issue(WebDriverCommand::SwitchToParentFrame).await?;
        Ok(self)
    }

    /// Sets the x, y, width, and height properties of the current window.
    ///
    /// See [10.7.2 Set Window Rect](https://www.w3.org/TR/webdriver1/#dfn-set-window-rect) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Set Window Rect"))]
    pub async fn set_window_rect(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<(), error::CmdError> {
        let cmd = WebDriverCommand::SetWindowRect(webdriver::command::WindowRectParameters {
            x: Some(x as i32),
            y: Some(y as i32),
            width: Some(width as i32),
            height: Some(height as i32),
        });

        self.issue(cmd).await?;
        Ok(())
    }

    /// Gets the x, y, width, and height properties of the current window.
    ///
    /// See [10.7.1 Get Window Rect](https://www.w3.org/TR/webdriver1/#dfn-get-window-rect) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Window Rect"))]
    pub async fn get_window_rect(&mut self) -> Result<(u64, u64, u64, u64), error::CmdError> {
        match self.issue(WebDriverCommand::GetWindowRect).await? {
            Json::Object(mut obj) => {
                let x = match obj.remove("x").and_then(|x| x.as_u64()) {
                    Some(x) => x,
                    None => return Err(error::CmdError::NotW3C(Json::Object(obj))),
                };

                let y = match obj.remove("y").and_then(|y| y.as_u64()) {
                    Some(y) => y,
                    None => return Err(error::CmdError::NotW3C(Json::Object(obj))),
                };

                let width = match obj.remove("width").and_then(|width| width.as_u64()) {
                    Some(width) => width,
                    None => return Err(error::CmdError::NotW3C(Json::Object(obj))),
                };

                let height = match obj.remove("height").and_then(|height| height.as_u64()) {
                    Some(height) => height,
                    None => return Err(error::CmdError::NotW3C(Json::Object(obj))),
                };

                Ok((x, y, width, height))
            }
            v => Err(error::CmdError::NotW3C(v)),
        }
    }

    /// Sets the x, y, width, and height properties of the current window.
    ///
    /// See [10.7.2 Set Window Rect](https://www.w3.org/TR/webdriver1/#dfn-set-window-rect) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Set Window Rect"))]
    pub async fn set_window_size(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<(), error::CmdError> {
        let cmd = WebDriverCommand::SetWindowRect(webdriver::command::WindowRectParameters {
            x: None,
            y: None,
            width: Some(width as i32),
            height: Some(height as i32),
        });

        self.issue(cmd).await?;
        Ok(())
    }

    /// Gets the width and height of the current window.
    ///
    /// See [10.7.1 Get Window Rect](https://www.w3.org/TR/webdriver1/#dfn-get-window-rect) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Window Rect"))]
    pub async fn get_window_size(&mut self) -> Result<(u64, u64), error::CmdError> {
        let (_, _, width, height) = self.get_window_rect().await?;
        Ok((width, height))
    }

    /// Sets the x, y, width, and height properties of the current window.
    ///
    /// See [10.7.2 Set Window Rect](https://www.w3.org/TR/webdriver1/#dfn-set-window-rect) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Set Window Rect"))]
    pub async fn set_window_position(&mut self, x: u32, y: u32) -> Result<(), error::CmdError> {
        let cmd = WebDriverCommand::SetWindowRect(webdriver::command::WindowRectParameters {
            x: Some(x as i32),
            y: Some(y as i32),
            width: None,
            height: None,
        });

        self.issue(cmd).await?;
        Ok(())
    }

    /// Gets the x and y top-left coordinate of the current window.
    ///
    /// See [10.7.1 Get Window Rect](https://www.w3.org/TR/webdriver1/#dfn-get-window-rect) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Window Rect"))]
    pub async fn get_window_position(&mut self) -> Result<(u64, u64), error::CmdError> {
        let (x, y, _, _) = self.get_window_rect().await?;
        Ok((x, y))
    }
}

/// [Element Retrieval](https://www.w3.org/TR/webdriver1/#element-retrieval)
impl Client {
    /// Find an element on the page that matches the given [`Locator`].
    ///
    /// See [12.2 Find Element](https://www.w3.org/TR/webdriver1/#find-element) of the WebDriver
    /// standard.
    #[cfg_attr(docsrs, doc(alias = "Find Element"))]
    pub async fn find(&mut self, search: Locator<'_>) -> Result<Element, error::CmdError> {
        self.by(search.into()).await
    }

    /// Find all elements on the page that match the given [`Locator`].
    ///
    /// See [12.3 Find Elements](https://www.w3.org/TR/webdriver1/#find-elements) of the WebDriver
    /// standard.
    #[cfg_attr(docsrs, doc(alias = "Find Elements"))]
    pub async fn find_all(&mut self, search: Locator<'_>) -> Result<Vec<Element>, error::CmdError> {
        let res = self
            .issue(WebDriverCommand::FindElements(search.into()))
            .await?;
        let array = self.parse_lookup_all(res)?;
        Ok(array
            .into_iter()
            .map(move |e| Element {
                client: self.clone(),
                element: e,
            })
            .collect())
    }

    /// Get the active element for this session.
    ///
    /// The "active" element is the `Element` within the DOM that currently has focus. This will
    /// often be an `<input>` or `<textarea>` element that currently has the text selection, or
    /// another input element such as a checkbox or radio button. Which elements are focusable
    /// depends on the platform and browser configuration.
    ///
    /// If no element has focus, the result may be the page body or a `NoSuchElement` error.
    ///
    /// See [12.6 Get Active Element](https://www.w3.org/TR/webdriver1/#dfn-get-active-element) of
    /// the WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Active Element"))]
    pub async fn active_element(&mut self) -> Result<Element, error::CmdError> {
        let res = self.issue(WebDriverCommand::GetActiveElement).await?;
        let e = self.parse_lookup(res)?;
        Ok(Element {
            client: self.clone(),
            element: e,
        })
    }

    /// Locate a form on the page.
    ///
    /// Through the returned `Form`, HTML forms can be filled out and submitted.
    pub async fn form(&mut self, search: Locator<'_>) -> Result<Form, error::CmdError> {
        let l = search.into();
        let res = self.issue(WebDriverCommand::FindElement(l)).await?;
        let f = self.parse_lookup(res)?;
        Ok(Form {
            client: self.clone(),
            form: f,
        })
    }
}

/// [Document Handling](https://www.w3.org/TR/webdriver1/#document-handling)
impl Client {
    /// Get the HTML source for the current page.
    ///
    /// See [15.1 Get Page Source](https://www.w3.org/TR/webdriver1/#dfn-get-page-source) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Get Page Source"))]
    pub async fn source(&mut self) -> Result<String, error::CmdError> {
        let src = self.issue(WebDriverCommand::GetPageSource).await?;
        if let Some(src) = src.as_str() {
            Ok(src.to_string())
        } else {
            Err(error::CmdError::NotW3C(src))
        }
    }

    /// Execute the given JavaScript `script` in the current browser session.
    ///
    /// `args` is available to the script inside the `arguments` array. Since `Element` implements
    /// `Serialize`, you can also provide serialized `Element`s as arguments, and they will
    /// correctly deserialize to DOM elements on the other side.
    ///
    /// To retrieve the value of a variable, `return` has to be used in the JavaScript code.
    ///
    /// See [15.2.1 Execute Script](https://www.w3.org/TR/webdriver1/#dfn-execute-script) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Execute Script"))]
    pub async fn execute(
        &mut self,
        script: &str,
        mut args: Vec<Json>,
    ) -> Result<Json, error::CmdError> {
        self.fixup_elements(&mut args);
        let cmd = webdriver::command::JavascriptCommandParameters {
            script: script.to_string(),
            args: Some(args),
        };

        self.issue(WebDriverCommand::ExecuteScript(cmd)).await
    }

    /// Execute the given async JavaScript `script` in the current browser session.
    ///
    /// The provided JavaScript has access to `args` through the JavaScript variable `arguments`.
    /// The `arguments` array also holds an additional element at the end that provides a completion callback
    /// for the asynchronous code.
    ///
    /// Since `Element` implements `Serialize`, you can also provide serialized `Element`s as arguments, and they will
    /// correctly deserialize to DOM elements on the other side.
    ///
    /// # Examples
    ///
    /// Call a web API from the browser and retrieve the value asynchronously
    ///
    /// ```ignore
    /// const JS: &'static str = r#"
    ///     const [date, callback] = arguments;
    ///
    ///     fetch(`http://weather.api/${date}/hourly`)
    ///     // whenever the HTTP Request completes,
    ///     // send the value back to the Rust context
    ///     .then(data => {
    ///         callback(data.json())
    ///     })
    /// "#;
    ///
    /// let weather = client.execute_async(JS, vec![date]).await?;
    /// ```
    ///
    /// See [15.2.2 Execute Async
    /// Script](https://www.w3.org/TR/webdriver1/#dfn-execute-async-script) of the WebDriver
    /// standard.
    #[cfg_attr(docsrs, doc(alias = "Execute Async Script"))]
    pub async fn execute_async(
        &mut self,
        script: &str,
        mut args: Vec<Json>,
    ) -> Result<Json, error::CmdError> {
        self.fixup_elements(&mut args);
        let cmd = webdriver::command::JavascriptCommandParameters {
            script: script.to_string(),
            args: Some(args),
        };

        self.issue(WebDriverCommand::ExecuteAsyncScript(cmd)).await
    }
}

/// [Screen Capture](https://www.w3.org/TR/webdriver1/#screen-capture)
impl Client {
    /// Get a PNG-encoded screenshot of the current page.
    ///
    /// See [19.1 Take Screenshot](https://www.w3.org/TR/webdriver1/#dfn-take-screenshot) of the
    /// WebDriver standard.
    #[cfg_attr(docsrs, doc(alias = "Take Screenshot"))]
    pub async fn screenshot(&mut self) -> Result<Vec<u8>, error::CmdError> {
        let src = self.issue(WebDriverCommand::TakeScreenshot).await?;
        if let Some(src) = src.as_str() {
            base64::decode(src).map_err(error::CmdError::ImageDecodeError)
        } else {
            Err(error::CmdError::NotW3C(src))
        }
    }

    /// Get a PNG-encoded screenshot of an element.
    ///
    /// See [19.2 Take Element
    /// Screenshot](https://www.w3.org/TR/webdriver1/#dfn-take-element-screenshot) of the WebDriver
    /// standard.
    #[cfg_attr(docsrs, doc(alias = "Take Element Screenshot"))]
    pub async fn screenshot_element(
        &mut self,
        element: Element,
    ) -> Result<Vec<u8>, error::CmdError> {
        let src = self
            .issue(WebDriverCommand::TakeElementScreenshot(element.element))
            .await?;
        if let Some(src) = src.as_str() {
            base64::decode(src).map_err(error::CmdError::ImageDecodeError)
        } else {
            Err(error::CmdError::NotW3C(src))
        }
    }
}

/// Operations that wait for a change on the page.
impl Client {
    /// Wait for the given function to return `true` before proceeding.
    ///
    /// This can be useful to wait for something to appear on the page before interacting with it.
    /// While this currently just spins and yields, it may be more efficient than this in the
    /// future. In particular, in time, it may only run `is_ready` again when an event occurs on
    /// the page.
    pub async fn wait_for<F, FF>(&mut self, mut is_ready: F) -> Result<(), error::CmdError>
    where
        F: FnMut(&mut Client) -> FF,
        FF: Future<Output = Result<bool, error::CmdError>>,
    {
        while !is_ready(self).await? {}
        Ok(())
    }

    /// Wait for the given element to be present on the page.
    ///
    /// This can be useful to wait for something to appear on the page before interacting with it.
    /// While this currently just spins and yields, it may be more efficient than this in the
    /// future. In particular, in time, it may only run `is_ready` again when an event occurs on
    /// the page.
    pub async fn wait_for_find(&mut self, search: Locator<'_>) -> Result<Element, error::CmdError> {
        let s: webdriver::command::LocatorParameters = search.into();
        loop {
            match self
                .by(webdriver::command::LocatorParameters {
                    using: s.using,
                    value: s.value.clone(),
                })
                .await
            {
                Ok(v) => break Ok(v),
                Err(error::CmdError::NoSuchElement(_)) => {}
                Err(e) => break Err(e),
            }
        }
    }

    /// Wait for the page to navigate to a new URL before proceeding.
    ///
    /// If the `current` URL is not provided, `self.current_url()` will be used. Note however that
    /// this introduces a race condition: the browser could finish navigating *before* we call
    /// `current_url()`, which would lead to an eternal wait.
    pub async fn wait_for_navigation(
        &mut self,
        current: Option<url::Url>,
    ) -> Result<(), error::CmdError> {
        let current = match current {
            Some(current) => current,
            None => self.current_url_().await?,
        };

        self.wait_for(move |c| {
            // TODO: get rid of this clone
            let current = current.clone();
            // TODO: and this one too
            let mut c = c.clone();
            async move { Ok(c.current_url().await? != current) }
        })
        .await
    }
}

/// Raw access to the WebDriver instance.
impl Client {
    /// Issue an HTTP request to the given `url` with all the same cookies as the current session.
    ///
    /// Calling this method is equivalent to calling `with_raw_client_for` with an empty closure.
    pub async fn raw_client_for(
        &mut self,
        method: Method,
        url: &str,
    ) -> Result<hyper::Response<hyper::Body>, error::CmdError> {
        self.with_raw_client_for(method, url, |req| req.body(hyper::Body::empty()).unwrap())
            .await
    }

    /// Build and issue an HTTP request to the given `url` with all the same cookies as the current
    /// session.
    ///
    /// Before the HTTP request is issued, the given `before` closure will be called with a handle
    /// to the `Request` about to be sent.
    pub async fn with_raw_client_for<F>(
        &mut self,
        method: Method,
        url: &str,
        before: F,
    ) -> Result<hyper::Response<hyper::Body>, error::CmdError>
    where
        F: FnOnce(http::request::Builder) -> hyper::Request<hyper::Body>,
    {
        let url = url.to_owned();
        // We need to do some trickiness here. GetCookies will only give us the cookies for the
        // *current* domain, whereas we want the cookies for `url`'s domain. So, we navigate to the
        // URL in question, fetch its cookies, and then navigate back. *Except* that we can't do
        // that either (what if `url` is some huge file?). So we *actually* navigate to some weird
        // url that's unlikely to exist on the target doamin, and which won't resolve into the
        // actual content, but will still give the same cookies.
        //
        // The fact that cookies can have /path and security constraints makes this even more of a
        // pain. /path in particular is tricky, because you could have a URL like:
        //
        //    example.com/download/some_identifier/ignored_filename_just_for_show
        //
        // Imagine if a cookie is set with path=/download/some_identifier. How do we get that
        // cookie without triggering a request for the (large) file? I don't know. Hence: TODO.
        let old_url = self.current_url_().await?;
        let url = old_url.clone().join(&url)?;
        let cookie_url = url.clone().join("/please_give_me_your_cookies")?;
        self.goto(cookie_url.as_str()).await?;

        // TODO: go back before we return if this call errors:
        let cookies = self.issue(WebDriverCommand::GetCookies).await?;
        if !cookies.is_array() {
            return Err(error::CmdError::NotW3C(cookies));
        }
        self.back().await?;
        let ua = self.get_ua().await?;

        // now add all the cookies
        let mut all_ok = true;
        let mut jar = Vec::new();
        for cookie in cookies.as_array().unwrap() {
            if !cookie.is_object() {
                all_ok = false;
                break;
            }

            // https://w3c.github.io/webdriver/webdriver-spec.html#cookies
            let cookie = cookie.as_object().unwrap();
            if !cookie.contains_key("name") || !cookie.contains_key("value") {
                all_ok = false;
                break;
            }

            if !cookie["name"].is_string() || !cookie["value"].is_string() {
                all_ok = false;
                break;
            }

            // Note that since we're sending these cookies, all that matters is the mapping
            // from name to value. The other fields only matter when deciding whether to
            // include a cookie or not, and the driver has already decided that for us
            // (GetCookies is for a particular URL).
            jar.push(
                cookie::Cookie::new(
                    cookie["name"].as_str().unwrap().to_owned(),
                    cookie["value"].as_str().unwrap().to_owned(),
                )
                .encoded()
                .to_string(),
            );
        }

        if !all_ok {
            return Err(error::CmdError::NotW3C(cookies));
        }

        let mut req = hyper::Request::builder();
        req = req
            .method(method)
            .uri(http::Uri::try_from(url.as_str()).unwrap());
        req = req.header(hyper::header::COOKIE, jar.join("; "));
        if let Some(s) = ua {
            req = req.header(hyper::header::USER_AGENT, s);
        }
        let req = before(req);
        let (tx, rx) = oneshot::channel();
        self.issue(Cmd::Raw { req, rsp: tx }).await?;
        match rx.await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => Err(e.into()),
            Err(e) => unreachable!("Session ended prematurely: {:?}", e),
        }
    }
}

/// Helper methods
impl Client {
    async fn by(
        &mut self,
        locator: webdriver::command::LocatorParameters,
    ) -> Result<Element, error::CmdError> {
        let res = self.issue(WebDriverCommand::FindElement(locator)).await?;
        let e = self.parse_lookup(res)?;
        Ok(Element {
            client: self.clone(),
            element: e,
        })
    }

    /// Extract the `WebElement` from a `FindElement` or `FindElementElement` command.
    pub(crate) fn parse_lookup(
        &self,
        res: Json,
    ) -> Result<webdriver::common::WebElement, error::CmdError> {
        let mut res = match res {
            Json::Object(o) => o,
            res => return Err(error::CmdError::NotW3C(res)),
        };

        // legacy protocol uses "ELEMENT" as identifier
        let key = if self.is_legacy() {
            "ELEMENT"
        } else {
            ELEMENT_KEY
        };

        if !res.contains_key(key) {
            return Err(error::CmdError::NotW3C(Json::Object(res)));
        }

        match res.remove(key) {
            Some(Json::String(wei)) => {
                return Ok(webdriver::common::WebElement(wei));
            }
            Some(v) => {
                res.insert(key.to_string(), v);
            }
            None => {}
        }

        Err(error::CmdError::NotW3C(Json::Object(res)))
    }

    /// Extract `WebElement`s from a `FindElements` or `FindElementElements` command.
    pub(crate) fn parse_lookup_all(
        &self,
        res: Json,
    ) -> Result<Vec<webdriver::common::WebElement>, error::CmdError> {
        let res = match res {
            Json::Array(a) => a,
            res => return Err(error::CmdError::NotW3C(res)),
        };

        let mut array = Vec::new();
        for json in res {
            let e = self.parse_lookup(json)?;
            array.push(e);
        }

        Ok(array)
    }

    pub(crate) fn fixup_elements(&self, args: &mut [Json]) {
        if self.is_legacy() {
            for arg in args {
                // the serialization of WebElement uses the W3C index,
                // but legacy implementations need us to use the "ELEMENT" index
                if let Json::Object(ref mut o) = *arg {
                    if let Some(wei) = o.remove(ELEMENT_KEY) {
                        o.insert("ELEMENT".to_string(), wei);
                    }
                }
            }
        }
    }
}
