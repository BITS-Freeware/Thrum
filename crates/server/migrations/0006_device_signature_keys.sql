-- Each device is bound to the MLS signature key of the first key package it
-- uploads. Later uploads must carry the same key, and clients compare group
-- leaves with it. NULL until the device uploads a key package.
ALTER TABLE devices ADD COLUMN signature_key BLOB;
