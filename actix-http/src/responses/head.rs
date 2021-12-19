//! Response head type and caching pool.

use std::{
    cell::{Ref, RefCell, RefMut},
    ops,
};

use crate::{
    header::HeaderMap, message::Flags, ConnectionType, Extensions, StatusCode, Version,
};

thread_local! {
    static RESPONSE_POOL: BoxedResponsePool = BoxedResponsePool::create();
}

#[derive(Debug)]
pub struct ResponseHead {
    pub version: Version,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub reason: Option<&'static str>,
    pub(crate) extensions: RefCell<Extensions>,
    flags: Flags,
}

impl ResponseHead {
    /// Create new instance of `ResponseHead` type
    #[inline]
    pub fn new(status: StatusCode) -> ResponseHead {
        ResponseHead {
            status,
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(12),
            reason: None,
            flags: Flags::empty(),
            extensions: RefCell::new(Extensions::new()),
        }
    }

    #[inline]
    /// Read the message headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[inline]
    /// Mutable reference to the message headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Message extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.extensions.borrow()
    }

    /// Mutable reference to a the message's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions.borrow_mut()
    }

    #[inline]
    /// Set connection type of the message
    pub fn set_connection_type(&mut self, ctype: ConnectionType) {
        match ctype {
            ConnectionType::Close => self.flags.insert(Flags::CLOSE),
            ConnectionType::KeepAlive => self.flags.insert(Flags::KEEP_ALIVE),
            ConnectionType::Upgrade => self.flags.insert(Flags::UPGRADE),
        }
    }

    #[inline]
    pub fn connection_type(&self) -> ConnectionType {
        if self.flags.contains(Flags::CLOSE) {
            ConnectionType::Close
        } else if self.flags.contains(Flags::KEEP_ALIVE) {
            ConnectionType::KeepAlive
        } else if self.flags.contains(Flags::UPGRADE) {
            ConnectionType::Upgrade
        } else if self.version < Version::HTTP_11 {
            ConnectionType::Close
        } else {
            ConnectionType::KeepAlive
        }
    }

    /// Check if keep-alive is enabled
    #[inline]
    pub fn keep_alive(&self) -> bool {
        self.connection_type() == ConnectionType::KeepAlive
    }

    /// Check upgrade status of this message
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.connection_type() == ConnectionType::Upgrade
    }

    /// Get custom reason for the response
    #[inline]
    pub fn reason(&self) -> &str {
        self.reason.unwrap_or_else(|| {
            self.status
                .canonical_reason()
                .unwrap_or("<unknown status code>")
        })
    }

    #[inline]
    pub(crate) fn conn_type(&self) -> Option<ConnectionType> {
        if self.flags.contains(Flags::CLOSE) {
            Some(ConnectionType::Close)
        } else if self.flags.contains(Flags::KEEP_ALIVE) {
            Some(ConnectionType::KeepAlive)
        } else if self.flags.contains(Flags::UPGRADE) {
            Some(ConnectionType::Upgrade)
        } else {
            None
        }
    }

    #[inline]
    /// Get response body chunking state
    pub fn chunked(&self) -> bool {
        !self.flags.contains(Flags::NO_CHUNKING)
    }

    #[inline]
    /// Set no chunking for payload
    pub fn no_chunking(&mut self, val: bool) {
        if val {
            self.flags.insert(Flags::NO_CHUNKING);
        } else {
            self.flags.remove(Flags::NO_CHUNKING);
        }
    }
}

pub(crate) struct BoxedResponseHead {
    head: Option<Box<ResponseHead>>,
}

impl BoxedResponseHead {
    /// Get new message from the pool of objects
    pub fn new(status: StatusCode) -> Self {
        RESPONSE_POOL.with(|p| p.get_message(status))
    }
}

impl ops::Deref for BoxedResponseHead {
    type Target = ResponseHead;

    fn deref(&self) -> &Self::Target {
        self.head.as_ref().unwrap()
    }
}

impl ops::DerefMut for BoxedResponseHead {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.head.as_mut().unwrap()
    }
}

impl Drop for BoxedResponseHead {
    fn drop(&mut self) {
        if let Some(head) = self.head.take() {
            RESPONSE_POOL.with(move |p| p.release(head))
        }
    }
}

/// Request's objects pool
#[doc(hidden)]
pub struct BoxedResponsePool(#[allow(clippy::vec_box)] RefCell<Vec<Box<ResponseHead>>>);

impl BoxedResponsePool {
    fn create() -> BoxedResponsePool {
        BoxedResponsePool(RefCell::new(Vec::with_capacity(128)))
    }

    /// Get message from the pool
    #[inline]
    fn get_message(&self, status: StatusCode) -> BoxedResponseHead {
        if let Some(mut head) = self.0.borrow_mut().pop() {
            head.reason = None;
            head.status = status;
            head.headers.clear();
            head.flags = Flags::empty();
            BoxedResponseHead { head: Some(head) }
        } else {
            BoxedResponseHead {
                head: Some(Box::new(ResponseHead::new(status))),
            }
        }
    }

    /// Release request instance
    #[inline]
    fn release(&self, mut msg: Box<ResponseHead>) {
        let pool = &mut self.0.borrow_mut();
        if pool.len() < 128 {
            msg.extensions.get_mut().clear();
            pool.push(msg);
        }
    }
}
