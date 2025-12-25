# Task 03a: Update Design Docs for Storage URL Format

## Goal
Update design documentation to clarify that `video_path`/`video_url` fields use a URL format to specify storage location, supporting both S3 and local filesystem.

## Dependencies
- Task 03: H.265 Converter (introduces the URL format)

## Files to Modify

```
docs/design_notes.md      # Add storage URL format section
docs/design_stage1.md     # Update protocol examples and types
```

---

## Changes Required

### 1. Add Storage URL Format section to design_notes.md

Add a new section explaining the URL format convention:

```markdown
# Storage URL Format

Video files are referenced using URLs that specify both the storage backend and path:

## Formats

- **S3**: `s3://bucket-name/path/to/video.h265`
- **Local filesystem**: `fs:///absolute/path/to/video.h265`

## Usage

- The `repo-cli convert` command generates these URLs in the offset JSON sidecar
- The streaming server parses these URLs to determine which `object_store` backend to use
- Clients send the URL in `SetVideo` messages

## Examples

```json
// S3 hosted video
{ "type": "SetVideo", "path": "s3://my-bucket/videos/clip001.h265" }

// Local development
{ "type": "SetVideo", "path": "fs:///workspace/data/clip001.h265" }
```
```

### 2. Update design_stage1.md protocol section

Update the `SetVideo` message examples to use URL format:

**Before:**
```json
{ "type": "SetVideo", "path": "videos/robot_cam_001.mp4" }
```

**After:**
```json
{ "type": "SetVideo", "path": "s3://bucket/videos/robot_cam_001.h265" }
```

### 3. Update FrameOffsets type documentation

Ensure the offset JSON format section shows `video_url` (not `video_path`):

```json
{
  "video_url": "s3://my-bucket/videos/robot_cam.h265",
  "frame_count": 150,
  "iraps": [...]
}
```

---

## Success Criteria

- [ ] `design_notes.md` has a "Storage URL Format" section
- [ ] All protocol examples use URL format (`s3://` or `fs://`)
- [ ] `design_stage1.md` examples are consistent with new format
- [ ] No references to bare relative paths remain in examples
