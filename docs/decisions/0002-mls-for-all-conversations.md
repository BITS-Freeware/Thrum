# 0002: MLS for All Conversations

Status: accepted

Every conversation, one-to-one or group, is an MLS group (RFC 9420), implemented with the `openmls` crate. One protocol covers both cases, handles many devices per user, and gives forward secrecy and post-compromise security. The project writes no cryptographic primitives of its own.

Considered instead: the Signal protocol (libsignal), which is proven for one-to-one chats but handles groups with a separate mechanism and is harder to build on outside its own apps.
